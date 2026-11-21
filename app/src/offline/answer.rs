//! The kept copy's part in a request: a kept answer is given at once, and asked for again afterwards once it is a
//! week old; one not kept is asked for, and kept if whole (`Call::keeps`). A mushaf page's listing or a juz's request
//! that is not kept itself is put together from the surahs' kept answers (`quran_core::offline`). When the network
//! fails, what is kept is what there is.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use quran_core::api::{Call, Client, Error, VerseQuery, VersesResponse};
use quran_core::feed::VerseSource;
use quran_core::lists::clean_juzs;
use super::store::{self, Kept, Store, holds};
use crate::net::Fetched;

/// A kept answer this old is asked for again (after it has been given).
pub const STALE: Duration = Duration::from_secs(7 * 24 * 60 * 60);
/// After asking again has failed (no connection), old answers are left alone this long.
const RETRY: Duration = Duration::from_secs(10 * 60);

/// Asks the API: a URL to its status and body.
pub type Fetch<'a> = &'a dyn Fn(&str) -> Fetched;

/// Asking again for old answers: one at a time (never holding up more than one network thread), and not for a while
/// after it has failed.
struct Renewal {
    asking: AtomicBool,
    failed: Mutex<Option<Instant>>,
}

impl Renewal {
    const fn new() -> Self {
        Self { asking: AtomicBool::new(false), failed: Mutex::new(None) }
    }
}

static RENEWAL: Renewal = Renewal::new();

/// Where answers come from: the kept copy (if there is one), the API, and the renewal of old answers.
#[derive(Clone, Copy)]
struct Sources<'a> {
    store: Option<&'a Store>,
    fetch: Fetch<'a>,
    renewal: &'a Renewal,
}

/// The API, blocking.
pub(super) fn api(url: &str) -> Fetched {
    crate::net::http().get(url, "application/json").map_err(|e| e.to_string())
}

fn user() -> Sources<'static> {
    Sources { store: store::user(), fetch: &api, renewal: &RENEWAL }
}

/// Gives `call`'s answer to `give` (on the network thread this runs on).
pub fn answer<T>(call: &Call<T>, give: impl FnOnce(Result<T, Error>)) {
    serve(user(), call, give);
}

/// Gives request `page` of a view's verses to `give` (on the network thread this runs on).
pub fn verses(client: &Client, source: VerseSource, query: &VerseQuery, page: u32, give: impl FnOnce(Result<VersesResponse, Error>)) {
    serve_verses(user(), client, source, query, page, give);
}

fn serve<T>(from: Sources, call: &Call<T>, give: impl FnOnce(Result<T, Error>)) {
    if let Some(store) = from.store
        && let Some((answer, kept)) = kept(store, call)
    {
        give(Ok(answer));
        renew(from, store, &call.url, &kept);
        return;
    }
    give(fetch_and_keep(from, call));
}

fn serve_verses(from: Sources, client: &Client, source: VerseSource, query: &VerseQuery, page: u32, give: impl FnOnce(Result<VersesResponse, Error>)) {
    let call = source.call(client, query, page);
    if let (Some(store), VerseSource::Page(_) | VerseSource::Juz(_)) = (from.store, source)
        && store.fetched(&call.url).is_none()
        && let Some((answer, old)) = from_surahs(store, client, source, query, page)
    {
        give(Ok(answer));
        for (url, kept) in &old {
            renew(from, store, url, kept);
        }
        return;
    }
    serve(from, &call, give);
}

/// The answer kept for `call`, read.
fn kept<T>(store: &Store, call: &Call<T>) -> Option<(T, Kept)> {
    let kept = store.read(&call.url).filter(|_| call.keeping())?;
    Some((call.read(200, &kept.body).ok()?, kept))
}

/// Asks for `call`'s answer, and keeps it if whole.
fn fetch_and_keep<T>(from: Sources, call: &Call<T>) -> Result<T, Error> {
    let (status, body) = (from.fetch)(&call.url).map_err(Error::Http)?;
    let answer = call.read(status, &body)?;
    if let Some(store) = from.store
        && call.keeps(&answer)
        && let Err(e) = store.write(&call.url, &body)
    {
        eprintln!("quran-eg: an answer could not be kept for reading offline: {e}");
    }
    Ok(answer)
}

/// A mushaf page's listing or a juz's request put together from the surahs' kept answers, and those of them that
/// are old.
fn from_surahs(store: &Store, client: &Client, source: VerseSource, query: &VerseQuery, page: u32) -> Option<(VersesResponse, Vec<(String, Kept)>)> {
    let juzs = match source {
        VerseSource::Juz(_) => clean_juzs(kept(store, &client.juzs_call())?.0),
        _ => Vec::new(),
    };
    let recipe = source.from_surahs(page, &juzs)?;
    let (mut answers, mut old) = (Vec::with_capacity(recipe.parts.len()), Vec::new());
    for &(chapter, request) in &recipe.parts {
        let call = VerseSource::Chapter { chapter, span: None }.call(client, query, request);
        let (answer, kept) = kept(store, &call)?;
        answers.push(answer);
        if kept.age() >= STALE {
            old.push((call.url, kept));
        }
    }
    Some((recipe.answer(answers)?, old))
}

/// Asks again for a kept answer that has grown old; the new one replaces it if it holds what it held (`holds`).
fn renew(from: Sources, store: &Store, url: &str, kept: &Kept) {
    let failed = |r: &Renewal| r.failed.lock().is_ok_and(|at| at.is_some_and(|at| at.elapsed() < RETRY));
    if kept.age() < STALE || failed(from.renewal) || from.renewal.asking.swap(true, Ordering::Acquire) {
        return;
    }
    match (from.fetch)(url) {
        Ok((200..=299, body)) if holds(&kept.body, &body) => {
            let _ = store.write(url, &body);
        }
        Ok(_) => {}
        Err(_) => {
            if let Ok(mut at) = from.renewal.failed.lock() {
                *at = Some(Instant::now());
            }
        }
    }
    from.renewal.asking.store(false, Ordering::Release);
}

#[cfg(test)]
pub mod tests {
    use std::cell::RefCell;

    use quran_core::reference::{verses_in, words_in};
    use quran_core::settings::Settings;
    use serde_json::json;

    use super::*;
    use crate::offline::store::tests::{backdate, remove, scratch};

    /// Request `request` of surah `c` as the API answers it, its first `missing` verses left out.
    pub fn surah(c: u16, request: u32, missing: usize) -> Vec<u8> {
        let n = u32::from(verses_in(c).unwrap());
        let verse = |v: u32| {
            let (v, count) = (v as u16, words_in(c, v as u16).unwrap() as u16);
            let word = |p: u16| json!({ "id": p, "position": p, "char_type_name": if p > count { "end" } else { "word" }, "text_uthmani": "ا" });
            json!({ "id": v, "verse_number": v, "verse_key": format!("{c}:{v}"), "words": (1..=count + 1).map(word).collect::<Vec<_>>() })
        };
        let verses: Vec<_> = ((request - 1) * 50 + 1..=(request * 50).min(n)).skip(missing).map(verse).collect();
        let pagination = json!({ "per_page": 50, "current_page": request, "next_page": null, "total_pages": n.div_ceil(50), "total_records": n });
        serde_json::to_vec(&json!({ "verses": verses, "pagination": pagination })).unwrap()
    }

    /// The API, answering `body` and noting what was asked; `None` is no connection.
    struct Api {
        body: RefCell<Option<Vec<u8>>>,
        asked: RefCell<Vec<String>>,
    }

    impl Api {
        fn new(body: Option<Vec<u8>>) -> Self {
            Self { body: RefCell::new(body), asked: RefCell::default() }
        }

        fn fetch(&self, url: &str) -> Fetched {
            self.asked.borrow_mut().push(url.to_string());
            self.body.borrow().clone().map(|b| (200, b)).ok_or_else(|| "no connection".into())
        }

        fn asked(&self) -> usize {
            self.asked.borrow().len()
        }
    }

    fn give<T>(answer: &mut Option<Result<T, Error>>) -> impl FnOnce(Result<T, Error>) + '_ {
        move |a| *answer = Some(a)
    }

    fn count(answer: Option<Result<VersesResponse, Error>>) -> usize {
        answer.unwrap().unwrap().verses.len()
    }

    #[test]
    fn an_answer_is_kept_if_whole_and_then_given_at_once() {
        let (store, renewal, client, query) = (scratch("answer"), Renewal::new(), Client::default(), Settings::default().verse_query());
        let call = VerseSource::Chapter { chapter: 112, span: None }.call(&client, &query, 1);
        let api = Api::new(Some(surah(112, 1, 0)));
        let from = Sources { store: Some(&store), fetch: &|url| api.fetch(url), renewal: &renewal };
        let mut answer = None;
        serve(from, &call, give(&mut answer));
        assert_eq!((count(answer), api.asked()), (4, 1));
        assert!(store.fetched(&call.url).is_some(), "kept");
        // Asked again: from the copy, even with no connection.
        api.body.replace(None);
        let mut answer = None;
        serve(from, &call, give(&mut answer));
        assert_eq!((count(answer), api.asked()), (4, 1));
        // An answer that lost a verse is given, not kept; a search is never kept.
        let short = VerseSource::Chapter { chapter: 113, span: None }.call(&client, &query, 1);
        api.body.replace(Some(surah(113, 1, 1)));
        let mut answer = None;
        serve(from, &short, give(&mut answer));
        assert_eq!(count(answer), 4);
        let search = client.search_call("light", 5, 1, "en");
        let mut found = None;
        serve(from, &search, give(&mut found));
        assert!(store.fetched(&short.url).is_none() && store.fetched(&search.url).is_none());
        // No connection and nothing kept: the error.
        api.body.replace(None);
        let mut answer = None;
        serve(from, &short, give(&mut answer));
        assert!(matches!(answer, Some(Err(Error::Http(_)))));
        remove(&store);
    }

    #[test]
    fn an_old_answer_is_given_then_renewed_if_the_new_one_holds_as_much() {
        let (store, renewal, client, query) = (scratch("renew"), Renewal::new(), Client::default(), Settings::default().verse_query());
        let call = VerseSource::Chapter { chapter: 2, span: None }.call(&client, &query, 6);
        store.write(&call.url, &surah(2, 6, 0)).unwrap();
        let api = Api::new(Some(surah(2, 6, 1)));
        let from = Sources { store: Some(&store), fetch: &|url| api.fetch(url), renewal: &renewal };
        let mut answer = None;
        serve(from, &call, give(&mut answer));
        assert_eq!((count(answer), api.asked()), (36, 0), "a week has not passed");
        backdate(&store, &call.url, 8);
        // Asked again, but the new answer lost a verse: the kept one stays (and is still old).
        let mut answer = None;
        serve(from, &call, give(&mut answer));
        assert_eq!((count(answer), api.asked()), (36, 1));
        assert!(store.read(&call.url).unwrap().age() > STALE);
        // A whole one replaces it.
        api.body.replace(Some(surah(2, 6, 0)));
        serve(from, &call, |_| {});
        assert!(api.asked() == 2 && store.read(&call.url).unwrap().age() < STALE);
        // With no connection, old answers are left alone for a while.
        backdate(&store, &call.url, 8);
        api.body.replace(None);
        serve(from, &call, |_| {});
        serve(from, &call, |_| {});
        assert_eq!(api.asked(), 3);
        remove(&store);
    }

    #[test]
    fn a_page_or_a_juz_is_put_together_from_the_kept_surahs() {
        let (store, renewal, client, query) = (scratch("together"), Renewal::new(), Client::default(), Settings::default().verse_query());
        let keep = |c: u16| store.write(&VerseSource::Chapter { chapter: c, span: None }.call(&client, &query, 1).url, &surah(c, 1, 0)).unwrap();
        let api = Api::new(None);
        let from = Sources { store: Some(&store), fetch: &|url| api.fetch(url), renewal: &renewal };
        // Page 589's second listing is 83:35 to 84:25: with 84 not kept, it is asked for.
        keep(83);
        let mut answer = None;
        serve_verses(from, &client, VerseSource::Page(589), &query, 2, give(&mut answer));
        assert!(api.asked.borrow()[0].contains("/verses/by_page/589?"));
        keep(84);
        let mut answer = None;
        serve_verses(from, &client, VerseSource::Page(589), &query, 2, give(&mut answer));
        assert_eq!((count(answer), api.asked()), (27, 1));
        // Juz 30's last request, from the kept juz list.
        let juz = |n: u8| {
            let mapping: serde_json::Map<_, _> = if n == 30 { (78..=114).map(|c| (c.to_string(), json!(format!("1-{}", verses_in(c).unwrap())))).collect() } else { Default::default() };
            json!({ "id": n, "juz_number": n, "verse_mapping": mapping })
        };
        store.write(&client.juzs_call().url, &serde_json::to_vec(&json!({ "juzs": (1..=30).map(juz).collect::<Vec<_>>() })).unwrap()).unwrap();
        (112..=114).for_each(keep);
        let mut answer = None;
        serve_verses(from, &client, VerseSource::Juz(30), &query, 12, give(&mut answer));
        assert_eq!((count(answer), api.asked()), (14, 1));
        remove(&store);
    }
}
