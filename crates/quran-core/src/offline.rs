//! Reading offline: which answers are whole enough to keep, and a mushaf page's listing or a juz's request put
//! together from the surahs' own answers (`/verses/by_chapter`, 50 verses a request), so that a copy of the 114
//! surahs serves every view.

use crate::api::{Juz, Pagination, Verse, VersesResponse};
use crate::feed::VerseSource;
use crate::integrity::words_structurally_ok;
use crate::parse::{CHAPTER_COUNT, PER_PAGE, api_page_of, juz_mapping};
use crate::reference;

/// A verse, as `(surah, ayah)`.
type Key = (u16, u16);

/// The verse after `key`, in mushaf order.
fn after((c, v): Key) -> Option<Key> {
    if v < reference::verses_in(c)? { Some((c, v + 1)) } else { (c < CHAPTER_COUNT).then_some((c + 1, 1)) }
}

/// The verses from `first` to `last`, as `(surah, from, to)` for each surah.
fn surahs(first: Key, last: Key) -> impl Iterator<Item = (u16, u16, u16)> {
    (first.0..=last.0).map(move |c| {
        let from = if c == first.0 { first.1 } else { 1 };
        let to = if c == last.0 { last.1 } else { reference::verses_in(c).unwrap_or(from) };
        (c, from, to)
    })
}

/// The requests that load a whole surah, 50 verses each.
pub fn surah_requests(chapter: u16) -> u32 {
    reference::verses_in(chapter).map_or(0, |n| api_page_of(n, PER_PAGE))
}

/// Whether an answer holds every verse its pagination counts for it, one after the other, each with all its words.
/// Only such answers are kept: one that lost a verse or a word is shown (with its notice) and asked for again.
pub fn whole(answer: &VersesResponse) -> bool {
    let p = &answer.pagination;
    let before = p.per_page.saturating_mul(p.current_page.saturating_sub(1));
    let count = p.per_page.min(p.total_records.saturating_sub(before)) as usize;
    let keys: Option<Vec<Key>> = answer.verses.iter().map(|v| Some((v.chapter_number()?, v.verse_number))).collect();
    keys.is_some_and(|keys| count > 0 && keys.len() == count && keys.windows(2).all(|w| after(w[0]) == Some(w[1])))
        && answer.verses.iter().all(words_structurally_ok)
}

/// A mushaf page's listing or a juz's request, put together from the surahs' answers.
#[derive(Debug, Clone, PartialEq)]
pub struct FromSurahs {
    /// The surahs' requests that hold its verses, as `(surah, request)`, in order.
    pub parts: Vec<(u16, u32)>,
    first: Key,
    last: Key,
    pagination: Pagination,
}

impl FromSurahs {
    fn new(first: Key, last: Key, pagination: Pagination) -> Self {
        let requests = |(c, from, to)| (api_page_of(from, PER_PAGE)..=api_page_of(to, PER_PAGE)).map(move |r| (c, r));
        Self { parts: surahs(first, last).flat_map(requests).collect(), first, last, pagination }
    }

    /// Its answer, from the answers to `parts`: the API's own but for the order of the verses' fields; `None` unless
    /// every verse is there, whole.
    pub fn answer(&self, parts: impl IntoIterator<Item = VersesResponse>) -> Option<VersesResponse> {
        let within = |v: &Verse| v.chapter_number().is_some_and(|c| (self.first..=self.last).contains(&(c, v.verse_number)));
        let verses = parts.into_iter().flat_map(|part| part.verses).filter(within).collect();
        let answer = VersesResponse { verses, pagination: self.pagination.clone() };
        whole(&answer).then_some(answer)
    }
}

impl VerseSource {
    /// Request `request` of a mushaf page or a juz, put together from the surahs' (a juz holds the verses `juzs` gives
    /// it). `None` for a surah's own requests, and for a request there is not.
    pub fn from_surahs(self, request: u32, juzs: &[Juz]) -> Option<FromSurahs> {
        let count = |first, last| surahs(first, last).map(|(_, from, to)| u32::from(to - from) + 1).sum::<u32>();
        match self {
            VerseSource::Chapter { .. } => None,
            VerseSource::Page(page) => {
                let listing = *reference::listings_for_page(page).get(usize::try_from(request).ok()?.checked_sub(1)?)?;
                let (first, last) = reference::listing_range(listing)?;
                let n = count(first, last);
                Some(FromSurahs::new(first, last, Pagination { per_page: n, current_page: 1, next_page: None, total_pages: 1, total_records: n }))
            }
            VerseSource::Juz(juz) => {
                let spans = juz_mapping(&juzs.iter().find(|j| j.juz_number == juz)?.verse_mapping);
                let total: u32 = spans.iter().map(|(_, s)| u32::from(s.to - s.from) + 1).sum();
                let skip = request.checked_sub(1)? * PER_PAGE;
                let take = PER_PAGE.min(total.checked_sub(skip).filter(|n| *n > 0)?);
                // The `i`-th verse of the juz.
                let at = |mut i: u32| {
                    spans.iter().find_map(|(c, s)| {
                        let n = u32::from(s.to - s.from) + 1;
                        let here = (i < n).then(|| (*c, s.from + i as u16));
                        i = i.saturating_sub(n);
                        here
                    })
                };
                let (first, last) = (at(skip)?, at(skip + take - 1)?);
                let pages = total.div_ceil(PER_PAGE);
                let next_page = (request < pages).then_some(request + 1);
                let pagination = Pagination { per_page: PER_PAGE, current_page: request, next_page, total_pages: pages, total_records: total };
                (count(first, last) == take).then(|| FromSurahs::new(first, last, pagination))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// A verse of `words` words and its marker.
    fn verse(c: u16, v: u16, words: u16) -> Verse {
        let mut list: Vec<_> = (1..=words).map(|p| json!({ "id": p, "position": p, "char_type_name": "word", "text_uthmani": "a" })).collect();
        list.push(json!({ "id": 0, "position": words + 1, "char_type_name": "end", "text_uthmani": "1" }));
        serde_json::from_value(json!({ "id": 1, "verse_number": v, "verse_key": format!("{c}:{v}"), "words": list })).unwrap()
    }

    /// Request `request` of a surah, as the API answers it.
    fn surah(c: u16, request: u32) -> VersesResponse {
        let n = reference::verses_in(c).unwrap();
        let from = (request - 1) * PER_PAGE + 1;
        let to = (request * PER_PAGE).min(u32::from(n));
        let verses = (from..=to).map(|v| verse(c, v as u16, 3)).collect();
        let next_page = (to < u32::from(n)).then_some(request + 1);
        let pagination = Pagination { per_page: PER_PAGE, current_page: request, next_page, total_pages: surah_requests(c), total_records: n.into() };
        VersesResponse { verses, pagination }
    }

    fn keys(answer: &VersesResponse) -> Vec<String> {
        answer.verses.iter().map(|v| v.verse_key.clone()).collect()
    }

    /// Juz 30 as the API lists it: 78:1-40 to 114:1-6.
    fn juz_30() -> Juz {
        let mapping: serde_json::Map<_, _> = (78..=114).map(|c| (c.to_string(), json!(format!("1-{}", reference::verses_in(c).unwrap())))).collect();
        serde_json::from_value(json!({ "id": 30, "juz_number": 30, "verse_mapping": mapping })).unwrap()
    }

    #[test]
    fn only_answers_with_every_verse_and_word_are_whole() {
        assert_eq!((surah_requests(1), surah_requests(2), surah_requests(114), surah_requests(115)), (1, 6, 1, 0));
        let total: u32 = (1..=CHAPTER_COUNT).map(surah_requests).sum();
        assert_eq!(total, 190, "a copy of every surah is 190 requests");
        let mut answer = surah(2, 6);
        assert!(whole(&answer) && answer.verses.len() == 36);
        // A verse missing, or the last one gone.
        let gone = answer.verses.remove(10);
        assert!(!whole(&answer));
        answer.verses.insert(10, gone);
        answer.verses.pop();
        assert!(!whole(&answer));
        // A verse that lost its marker.
        let mut answer = surah(1, 1);
        answer.verses[3].words.pop();
        assert!(!whole(&answer));
        assert!(!whole(&VersesResponse { verses: Vec::new(), pagination: Pagination::default() }));
    }

    #[test]
    fn a_page_listing_is_put_together_from_its_surahs() {
        // Page 589 is read from listings 588 and 589; the second is 83:35 to 84:25, 27 verses, as the API answers it.
        let listing = VerseSource::Page(589).from_surahs(2, &[]).unwrap();
        assert_eq!(listing.parts, [(83, 1), (84, 1)]);
        let answer = listing.answer([surah(83, 1), surah(84, 1)]).unwrap();
        assert_eq!((answer.verses.len(), answer.verses[0].verse_key.as_str(), keys(&answer).last().unwrap().as_str()), (27, "83:35", "84:25"));
        assert_eq!(answer.pagination, Pagination { per_page: 27, current_page: 1, next_page: None, total_pages: 1, total_records: 27 });
        assert_eq!(VerseSource::Page(589).from_surahs(1, &[]).unwrap().parts, [(83, 1)]);
        // A surah part missing or short: no answer.
        assert_eq!(listing.answer([surah(83, 1)]), None);
        let mut short = surah(84, 1);
        short.verses.remove(3);
        assert_eq!(listing.answer([surah(83, 1), short]), None);
        // No third listing; a surah's requests are its own.
        assert_eq!(VerseSource::Page(589).from_surahs(3, &[]), None);
        assert_eq!(VerseSource::Page(589).from_surahs(0, &[]), None);
        assert_eq!(VerseSource::Chapter { chapter: 2, span: None }.from_surahs(1, &[]), None);
        // Listing 41 is 2:249-252, across Al-Baqarah's requests 5 (to 2:250) and 6.
        let k = reference::listings_for_page(41).iter().position(|l| *l == 41).unwrap() as u32 + 1;
        assert_eq!(VerseSource::Page(41).from_surahs(k, &[]).unwrap().parts, [(2, 5), (2, 6)]);
    }

    #[test]
    fn a_juz_request_is_put_together_from_its_surahs() {
        let juzs = [juz_30()];
        // The last of its 12 requests: 112:2 to 114:6, 14 verses, as the API answers it.
        let last = VerseSource::Juz(30).from_surahs(12, &juzs).unwrap();
        assert_eq!(last.parts, [(112, 1), (113, 1), (114, 1)]);
        let answer = last.answer([surah(112, 1), surah(113, 1), surah(114, 1)]).unwrap();
        assert_eq!((answer.verses.len(), keys(&answer)[0].as_str()), (14, "112:2"));
        assert_eq!(answer.pagination, Pagination { per_page: 50, current_page: 12, next_page: None, total_pages: 12, total_records: 564 });
        // The first: 78:1-40 and 79:1-10.
        let first = VerseSource::Juz(30).from_surahs(1, &juzs).unwrap();
        assert_eq!(first.parts, [(78, 1), (79, 1)]);
        let answer = first.answer([surah(78, 1), surah(79, 1)]).unwrap();
        assert_eq!((answer.verses.len(), keys(&answer)[49].as_str(), answer.pagination.next_page), (50, "79:10", Some(2)));
        // No 13th request, and no juz the list doesn't have.
        assert_eq!(VerseSource::Juz(30).from_surahs(13, &juzs), None);
        assert_eq!(VerseSource::Juz(29).from_surahs(1, &juzs), None);
    }
}
