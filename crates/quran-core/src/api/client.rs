//! The API client: what to ask for (a [`Call`]: a URL and how to read the
//! answer) and, on native targets, asking for it.
//!
//! Native requests block (`ureq`): the app runs them on worker threads that
//! hand their result back over a channel and wake the UI — no async runtime in
//! the binary. A browser can't block, so there the app sends a [`Call`] with
//! `fetch` and gives the answer to [`Call::read`].

use super::{DEFAULT_BASE_URL, query::VerseQuery, types::*};
use crate::integrity::words_structurally_ok;
use crate::offline::whole;
use crate::parse::{CHAPTER_COUNT, JUZ_COUNT};
use serde::de::DeserializeOwned;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("network error: {0}")]
    Http(String),
    #[error("API returned HTTP {status}: {body}")]
    Status { status: u16, body: String },
    #[error("could not parse API response: {0}")]
    Decode(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

/// One request to the API: where it goes, how its answer is read, and which answers may be kept for reading offline.
#[derive(Debug, Clone)]
pub struct Call<T> {
    pub url: String,
    parse: fn(&str) -> Result<T>,
    /// What an answer must be to be kept (whole: every verse, every surah); `None`: answers are not kept (a search's).
    keep: Option<fn(&T) -> bool>,
}

impl<T> Call<T> {
    /// Read the answer: a non-2xx status is an error that quotes the body.
    pub fn read(&self, status: u16, body: &[u8]) -> Result<T> {
        let body = String::from_utf8_lossy(body);
        if !(200..300).contains(&status) {
            return Err(Error::Status { status, body: truncate(&body, 300) });
        }
        (self.parse)(&body)
    }

    /// Whether its answers are ever kept.
    pub fn keeping(&self) -> bool {
        self.keep.is_some()
    }

    /// Whether this answer may be kept.
    pub fn keeps(&self, answer: &T) -> bool {
        self.keep.is_some_and(|keep| keep(answer))
    }
}

/// Any answer that could be read.
fn read_at_all<T>(_: &T) -> bool {
    true
}

#[derive(Debug, Clone)]
pub struct Client {
    base_url: String,
    #[cfg(not(target_arch = "wasm32"))]
    http: crate::http::Http,
}

impl Default for Client {
    fn default() -> Self {
        Self::new(DEFAULT_BASE_URL)
    }
}

impl PartialEq for Client {
    fn eq(&self, other: &Self) -> bool {
        self.base_url == other.base_url
    }
}

impl Client {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            // Every request has deadlines, so a hung connection turns into a visible error.
            #[cfg(not(target_arch = "wasm32"))]
            http: crate::http::Http::new(std::time::Duration::from_secs(15), std::time::Duration::from_secs(45)),
        }
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Build the full URL for `path` (must start with `/`) and query pairs.
    pub fn url(&self, path: &str, query: &[(&str, String)]) -> String {
        let mut url = format!("{}{}", self.base_url, path);
        if !query.is_empty() {
            url.push('?');
            let encoded: Vec<String> = query
                .iter()
                .map(|(k, v)| format!("{}={}", encode(k), encode(v)))
                .collect();
            url.push_str(&encoded.join("&"));
        }
        url
    }

    fn call<T>(&self, path: &str, query: &[(&str, String)], parse: fn(&str) -> Result<T>, keep: Option<fn(&T) -> bool>) -> Call<T> {
        Call { url: self.url(path, query), parse, keep }
    }

    /// All 114 chapters, names translated into `language`.
    pub fn chapters_call(&self, language: &str) -> Call<Vec<Chapter>> {
        let every = |list: &Vec<Chapter>| list.len() == usize::from(CHAPTER_COUNT);
        self.call("/chapters", &[("language", language.into())], |b| Ok(parse_json::<ChaptersResponse>(b)?.chapters), Some(every))
    }

    pub fn chapter_call(&self, id: u16, language: &str) -> Call<Chapter> {
        let query = [("language", language.into())];
        self.call(&format!("/chapters/{id}"), &query, |b| Ok(parse_json::<ChapterResponse>(b)?.chapter), Some(read_at_all))
    }

    pub fn verses_by_chapter_call(&self, chapter: u16, q: &VerseQuery) -> Call<VersesResponse> {
        self.call(&format!("/verses/by_chapter/{chapter}"), &q.to_pairs(), parse_json, Some(whole))
    }

    pub fn verses_by_page_call(&self, page: u16, q: &VerseQuery) -> Call<VersesResponse> {
        self.call(&format!("/verses/by_page/{page}"), &q.to_pairs(), parse_json, Some(whole))
    }

    pub fn verses_by_juz_call(&self, juz: u8, q: &VerseQuery) -> Call<VersesResponse> {
        self.call(&format!("/verses/by_juz/{juz}"), &q.to_pairs(), parse_json, Some(whole))
    }

    /// Single verse, `key` like `"2:255"`.
    pub fn verse_by_key_call(&self, key: &str, q: &VerseQuery) -> Call<Verse> {
        self.call(&format!("/verses/by_key/{key}"), &q.to_pairs(), |b| Ok(parse_json::<VerseResponse>(b)?.verse), Some(words_structurally_ok))
    }

    pub fn juzs_call(&self) -> Call<Vec<Juz>> {
        // (The API has been known to list a juz twice: each must be there.)
        let every = |list: &Vec<Juz>| (1..=JUZ_COUNT).all(|n| list.iter().any(|j| j.juz_number == n));
        self.call("/juzs", &[], |b| Ok(parse_json::<JuzsResponse>(b)?.juzs), Some(every))
    }

    /// Available translations; names localized into `language`.
    pub fn translations_call(&self, language: &str) -> Call<Vec<TranslationResource>> {
        let query = [("language", language.into())];
        self.call("/resources/translations", &query, |b| Ok(parse_json::<TranslationsResponse>(b)?.translations), Some(|list| !list.is_empty()))
    }

    pub fn languages_call(&self, language: &str) -> Call<Vec<Language>> {
        let query = [("language", language.into())];
        self.call("/resources/languages", &query, |b| Ok(parse_json::<LanguagesResponse>(b)?.languages), Some(|list| !list.is_empty()))
    }

    pub fn footnote_call(&self, id: u32) -> Call<FootNote> {
        self.call(&format!("/foot_notes/{id}"), &[], |b| Ok(parse_json::<FootNoteResponse>(b)?.foot_note), Some(read_at_all))
    }

    /// Full-text search. `page` is 1-based.
    pub fn search_call(&self, query: &str, size: u32, page: u32, language: &str) -> Call<Search> {
        let query =
            [("q", query.into()), ("size", size.to_string()), ("page", page.to_string()), ("language", language.into())];
        // Searches are not kept.
        self.call("/search", &query, |b| Ok(parse_json::<SearchResponse>(b)?.search), None)
    }
}

/// Asking, on targets that can block.
#[cfg(not(target_arch = "wasm32"))]
impl Client {
    /// Send a call and read its answer (blocking).
    pub fn send<T>(&self, call: &Call<T>) -> Result<T> {
        let (status, body) = self.http.get(&call.url, "application/json").map_err(|e| Error::Http(e.to_string()))?;
        call.read(status, &body)
    }

    pub fn chapters(&self, language: &str) -> Result<Vec<Chapter>> {
        self.send(&self.chapters_call(language))
    }

    pub fn chapter(&self, id: u16, language: &str) -> Result<Chapter> {
        self.send(&self.chapter_call(id, language))
    }

    pub fn verses_by_chapter(&self, chapter: u16, q: &VerseQuery) -> Result<VersesResponse> {
        self.send(&self.verses_by_chapter_call(chapter, q))
    }

    pub fn verses_by_page(&self, page: u16, q: &VerseQuery) -> Result<VersesResponse> {
        self.send(&self.verses_by_page_call(page, q))
    }

    pub fn verses_by_juz(&self, juz: u8, q: &VerseQuery) -> Result<VersesResponse> {
        self.send(&self.verses_by_juz_call(juz, q))
    }

    pub fn verse_by_key(&self, key: &str, q: &VerseQuery) -> Result<Verse> {
        self.send(&self.verse_by_key_call(key, q))
    }

    pub fn juzs(&self) -> Result<Vec<Juz>> {
        self.send(&self.juzs_call())
    }

    pub fn translations(&self, language: &str) -> Result<Vec<TranslationResource>> {
        self.send(&self.translations_call(language))
    }

    pub fn languages(&self, language: &str) -> Result<Vec<Language>> {
        self.send(&self.languages_call(language))
    }

    pub fn footnote(&self, id: u32) -> Result<FootNote> {
        self.send(&self.footnote_call(id))
    }

    pub fn search(&self, query: &str, size: u32, page: u32, language: &str) -> Result<Search> {
        self.send(&self.search_call(query, size, page, language))
    }
}

/// Parse an API response body. The API repeats some keys in an object when a
/// field it sends by default is also requested explicitly (e.g. `line_number` in
/// `word_fields`), and derived serde types reject duplicate fields. Going through
/// `serde_json::Value` first keeps the last value of a repeated key, as browsers'
/// `JSON.parse` does, so such responses still load.
pub fn parse_json<T: DeserializeOwned>(body: &str) -> Result<T> {
    let value: serde_json::Value = serde_json::from_str(body)?;
    Ok(serde_json::from_value(value)?)
}

/// Minimal application/x-www-form-urlencoded-style percent encoding (RFC 3986 unreserved kept).
pub fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b',' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_urls() {
        let c = Client::new("https://example.com/api/v4/");
        assert_eq!(
            c.url("/search", &[("q", "light & dark".into()), ("size", "5".into())]),
            "https://example.com/api/v4/search?q=light%20%26%20dark&size=5"
        );
    }

    #[test]
    fn whole_answers_are_kept_and_searches_never() {
        let c = Client::default();
        assert!(!c.search_call("light", 5, 1, "en").keeping());
        let chapters = c.chapters_call("en");
        assert!(chapters.keeping() && !chapters.keeps(&Vec::new()));
        let juzs = c.juzs_call();
        let juz = |n: u8| Juz { id: n.into(), juz_number: n, verse_mapping: Default::default(), first_verse_id: None, last_verse_id: None, verses_count: None };
        // Listed twice over, as the API has done, they are all there; one short, not.
        let twice: Vec<Juz> = (1..=30).chain(1..=30).map(juz).collect();
        assert!(juzs.keeps(&twice) && !juzs.keeps(&twice[1..30].to_vec()));
        assert!(c.footnote_call(1).keeping() && !c.translations_call("en").keeps(&Vec::new()));
    }

    #[test]
    fn encodes_arabic() {
        assert_eq!(encode("نور"), "%D9%86%D9%88%D8%B1");
    }
}
