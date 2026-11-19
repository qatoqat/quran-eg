//! HTTP client. Blocking (`ureq`): an egui app has no async runtime of its own,
//! so requests run on plain worker threads that hand their result back over a
//! channel and wake the UI. That keeps tokio and an async HTTP stack out of the
//! binary.

use super::{DEFAULT_BASE_URL, query::VerseQuery, types::*};
use crate::http::Http;
use serde::de::DeserializeOwned;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("network error: {0}")]
    Http(#[from] Box<ureq::Error>),
    #[error("API returned HTTP {status}: {body}")]
    Status { status: u16, body: String },
    #[error("could not parse API response: {0}")]
    Decode(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone)]
pub struct Client {
    base_url: String,
    http: Http,
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
            http: build_http(),
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

    fn get<T: DeserializeOwned>(&self, path: &str, query: &[(&str, String)]) -> Result<T> {
        let (status, body) = self.http.get(&self.url(path, query), "application/json")?;
        let body = String::from_utf8_lossy(&body);
        if !(200..300).contains(&status) {
            return Err(Error::Status { status, body: truncate(&body, 300) });
        }
        parse_json(&body)
    }

    /// All 114 chapters, names translated into `language`.
    pub fn chapters(&self, language: &str) -> Result<Vec<Chapter>> {
        let r: ChaptersResponse = self.get("/chapters", &[("language", language.into())])?;
        Ok(r.chapters)
    }

    pub fn chapter(&self, id: u16, language: &str) -> Result<Chapter> {
        let r: ChapterResponse =
            self.get(&format!("/chapters/{id}"), &[("language", language.into())])?;
        Ok(r.chapter)
    }

    pub fn verses_by_chapter(&self, chapter: u16, q: &VerseQuery) -> Result<VersesResponse> {
        self.get(&format!("/verses/by_chapter/{chapter}"), &q.to_pairs())
    }

    pub fn verses_by_page(&self, page: u16, q: &VerseQuery) -> Result<VersesResponse> {
        self.get(&format!("/verses/by_page/{page}"), &q.to_pairs())
    }

    pub fn verses_by_juz(&self, juz: u8, q: &VerseQuery) -> Result<VersesResponse> {
        self.get(&format!("/verses/by_juz/{juz}"), &q.to_pairs())
    }

    /// Single verse, `key` like `"2:255"`.
    pub fn verse_by_key(&self, key: &str, q: &VerseQuery) -> Result<Verse> {
        let r: VerseResponse = self.get(&format!("/verses/by_key/{key}"), &q.to_pairs())?;
        Ok(r.verse)
    }

    pub fn juzs(&self) -> Result<Vec<Juz>> {
        let r: JuzsResponse = self.get("/juzs", &[])?;
        Ok(r.juzs)
    }

    /// Available translations; names localized into `language`.
    pub fn translations(&self, language: &str) -> Result<Vec<TranslationResource>> {
        let r: TranslationsResponse =
            self.get("/resources/translations", &[("language", language.into())])?;
        Ok(r.translations)
    }

    pub fn languages(&self, language: &str) -> Result<Vec<Language>> {
        let r: LanguagesResponse =
            self.get("/resources/languages", &[("language", language.into())])?;
        Ok(r.languages)
    }

    pub fn footnote(&self, id: u32) -> Result<FootNote> {
        let r: FootNoteResponse = self.get(&format!("/foot_notes/{id}"), &[])?;
        Ok(r.foot_note)
    }

    /// Full-text search. `page` is 1-based.
    pub fn search(&self, query: &str, size: u32, page: u32, language: &str) -> Result<Search> {
        let r: SearchResponse = self
            .get(
                "/search",
                &[
                    ("q", query.into()),
                    ("size", size.to_string()),
                    ("page", page.to_string()),
                    ("language", language.into()),
                ],
            )
            ?;
        Ok(r.search)
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

fn build_http() -> Http {
    Http::new(std::time::Duration::from_secs(15), std::time::Duration::from_secs(45))
}

/// Minimal application/x-www-form-urlencoded-style percent encoding (RFC 3986 unreserved kept).
fn encode(s: &str) -> String {
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
    fn encodes_arabic() {
        assert_eq!(encode("نور"), "%D9%86%D9%88%D8%B1");
    }
}
