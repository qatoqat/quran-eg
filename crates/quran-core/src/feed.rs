//! Paged verse loading for a reader view: 50 verses per API page, merged in
//! order and checked. This is the state and its rules; the app runs the
//! requests on a worker thread and feeds the responses in.

use crate::api::{Client, Error, Verse, VerseQuery, VersesResponse};
use crate::integrity::{self, IntegrityIssue};
use crate::parse::{PER_PAGE, VerseSpan, api_page_of, api_pages_for, next_page_within};
use crate::reference;
use crate::util::short_list;

/// What to load.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerseSource {
    Chapter { chapter: u16, span: Option<VerseSpan> },
    Page(u16),
    Juz(u8),
}

impl VerseSource {
    pub fn first_api_page(self) -> u32 {
        match self {
            VerseSource::Chapter { span: Some(s), .. } => api_page_of(s.from, PER_PAGE),
            _ => 1,
        }
    }

    /// Last request worth making, when known up front. For a mushaf page the
    /// requests are its listings (see `reference::listings_for_page`), one each.
    pub fn last_api_page(self) -> Option<u32> {
        match self {
            VerseSource::Chapter { span: Some(s), .. } => Some(*api_pages_for(s, PER_PAGE).end()),
            VerseSource::Page(p) => Some(reference::listings_for_page(p).len().max(1) as u32),
            _ => None,
        }
    }

    /// The request after `page`, given what the API says comes next.
    pub fn next_api_page(self, page: u32, api_next: Option<u32>) -> Option<u32> {
        match self {
            VerseSource::Page(_) => Some(page + 1).filter(|n| Some(*n) <= self.last_api_page()),
            _ => next_page_within(api_next, self.last_api_page()).filter(|n| *n > page),
        }
    }

    /// A mushaf page is one unit: all its listings are fetched right away, not
    /// when the reader scrolls to the end.
    pub fn loads_at_once(self) -> bool {
        matches!(self, VerseSource::Page(_))
    }

    pub fn includes(self, verse: &Verse) -> bool {
        match self {
            VerseSource::Chapter { span: Some(s), .. } => s.contains(verse.verse_number),
            // A neighbouring listing is fetched for the verses printed on this page only.
            VerseSource::Page(p) => verse.chapter_number().is_some_and(|c| {
                reference::verses_on_page(p).iter().any(|(chapter, span)| *chapter == c && span.contains(verse.verse_number))
            }),
            _ => true,
        }
    }

    /// Fetch API page `page` of this source (blocking).
    pub fn fetch(self, client: &Client, query: &VerseQuery, page: u32) -> Result<VersesResponse, Error> {
        let mut q = query.with_page(page);
        q.per_page = PER_PAGE;
        match self {
            VerseSource::Chapter { chapter, .. } => client.verses_by_chapter(chapter, &q),
            VerseSource::Page(p) => {
                // Request `page` of a mushaf page is its `page`-th listing, which fits one API page.
                let listings = reference::listings_for_page(p);
                let listing = listings.get(page as usize - 1).copied().unwrap_or(p);
                let response = client.verses_by_page(listing, &q.with_page(1))?;
                if response.pagination.next_page.is_some() {
                    return Err(Error::Status {
                        status: 0,
                        body: format!("the listing of page {listing} has more verses than one request returns"),
                    });
                }
                Ok(response)
            }
            VerseSource::Juz(j) => client.verses_by_juz(j, &q),
        }
    }
}

/// The verses of a source loaded so far.
#[derive(Debug, Clone, PartialEq)]
pub struct Feed {
    pub source: VerseSource,
    /// Sorted, without duplicates.
    pub verses: Vec<Verse>,
    /// Next API page to fetch; `None` once everything is loaded.
    pub next_page: Option<u32>,
    /// The API's own count of the source's verses (pages and ajzāʾ; a chapter
    /// range is checked against the range instead).
    pub total: Option<u32>,
    /// Problems found while merging pages (duplicates, ordering).
    pub merge_issues: Vec<IntegrityIssue>,
}

impl Feed {
    pub fn new(source: VerseSource) -> Self {
        Self {
            source,
            verses: Vec::new(),
            next_page: Some(source.first_api_page()),
            total: None,
            merge_issues: Vec::new(),
        }
    }

    /// Everything has been loaded.
    pub fn complete(&self) -> bool {
        self.next_page.is_none()
    }

    /// Merge the response to API page `page`.
    pub fn merge(&mut self, page: u32, resp: VersesResponse) {
        let last = self.source.last_api_page();
        // Guard against an API that keeps returning the same page.
        self.next_page = self.source.next_api_page(page, resp.pagination.next_page).filter(|_| !resp.verses.is_empty());
        if last.is_none() {
            self.total = Some(resp.pagination.total_records);
        }
        let source = self.source;
        self.verses.extend(resp.verses.into_iter().filter(|v| source.includes(v)));
        self.merge_issues.extend(integrity::sort_dedup(&mut self.verses));
    }

    /// Integrity of what is loaded so far; `expected` (every verse of the view)
    /// is checked only once everything has loaded.
    pub fn issues(&self, expected: &[(u16, VerseSpan)]) -> Vec<IntegrityIssue> {
        let expected = if self.complete() { expected } else { &[] };
        let mut issues = self.merge_issues.clone();
        // The mushaf always shows Uthmani (QCF glyphs / KFGQPC Hafs text).
        issues.extend(integrity::check_all(&self.verses, "text_uthmani", expected, reference::verses_in));
        // Lines: a mushaf page must be whole once it has loaded; elsewhere every
        // line between the first and the last one loaded.
        match self.source {
            VerseSource::Page(p) if self.complete() => issues.extend(integrity::line_issues(&self.verses, Some(p))),
            VerseSource::Page(_) => {}
            _ => issues.extend(integrity::line_issues(&self.verses, None)),
        }
        issues
    }

    /// Once loading has finished: `(shown, total)` if fewer verses arrived than
    /// the API itself counts for the page/juz.
    pub fn short_by(&self) -> Option<(usize, u32)> {
        match self.total {
            Some(t) if self.complete() && (self.verses.len() as u32) < t => Some((self.verses.len(), t)),
            _ => None,
        }
    }

    pub fn has_verse(&self, verse_key: &str) -> bool {
        self.verses.iter().any(|v| v.verse_key == verse_key)
    }
}

/// The warning banner for a list of issues: one line per kind of problem, each
/// naming the verses. Empty when there is nothing to report. The flag says whether
/// reloading could help (verses missing from the response).
pub fn issue_messages(issues: &[IntegrityIssue]) -> (Vec<String>, bool) {
    let keys = |pred: &dyn Fn(&IntegrityIssue) -> bool| integrity::keys_where(issues, pred);
    let missing = keys(&|i| matches!(i, IntegrityIssue::MissingVerse { .. }));
    // (A short line is named by its page and line, not by a verse.)
    let short_lines: Vec<String> =
        issues.iter().filter(|i| matches!(i, IntegrityIssue::LineWordCount { .. })).map(ToString::to_string).collect();
    let reload = !missing.is_empty() || !short_lines.is_empty();
    let groups: [(&str, Vec<String>); 8] = [
        (
            "Some verse data looked incomplete (words missing or out of place); showing full verse text instead for: ",
            keys(&IntegrityIssue::is_structural_word_issue),
        ),
        (
            "Some words came without Unicode text (marked ⟨?⟩ unless the mushaf font draws them) in: ",
            keys(&|i| matches!(i, IntegrityIssue::EmptyWordText { .. })),
        ),
        (
            "Mushaf line data was missing for these verses; they are shown as flowing text: ",
            keys(&IntegrityIssue::is_layout_issue),
        ),
        (
            "The mushaf page layout looked inconsistent near (all words are still shown): ",
            keys(&IntegrityIssue::is_page_shape_issue),
        ),
        (
            "Some words are shown as Unicode text because their glyph font page did not match: ",
            keys(&|i| matches!(i, IntegrityIssue::GlyphPageMismatch { .. })),
        ),
        (
            "The server returned verse numbers that do not exist: ",
            keys(&|i| matches!(i, IntegrityIssue::UnknownVerse { .. })),
        ),
        ("Some verses were missing from the server response: ", missing),
        ("Some mushaf lines do not have all their words: ", short_lines),
    ];
    let lines = groups
        .into_iter()
        .filter(|(_, keys)| !keys.is_empty())
        .map(|(text, keys)| format!("{text}{}", short_list(&keys, 12)))
        .collect();
    (lines, reload)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::Pagination;
    use serde_json::json;

    fn verse(key: &str) -> Verse {
        let n: u16 = key.split(':').nth(1).unwrap().parse().unwrap();
        serde_json::from_value(json!({
            "id": 1, "verse_number": n, "verse_key": key,
            "text_uthmani": "a",
            "words": [
                { "id": 1, "position": 1, "char_type_name": "word", "text_uthmani": "a",
                  "page_number": 1, "line_number": n, "code_v2": "x", "v2_page": 1 },
                { "id": 2, "position": 2, "char_type_name": "end", "text_uthmani": "1",
                  "page_number": 1, "line_number": n, "code_v2": "y", "v2_page": 1 }
            ]
        }))
        .unwrap()
    }

    fn response(keys: &[&str], next: Option<u32>, total: u32) -> VersesResponse {
        VersesResponse {
            verses: keys.iter().map(|k| verse(k)).collect(),
            pagination: Pagination { per_page: 50, current_page: 1, next_page: next, total_pages: 2, total_records: total },
        }
    }

    #[test]
    fn source_pages() {
        let s = VerseSource::Chapter { chapter: 2, span: Some(VerseSpan { from: 120, to: 160 }) };
        assert_eq!(s.first_api_page(), 3);
        assert_eq!(s.last_api_page(), Some(4));
        assert!(s.includes(&verse("2:120")) && s.includes(&verse("2:160")));
        assert!(!s.includes(&verse("2:119")) && !s.includes(&verse("2:161")));
        let whole = VerseSource::Chapter { chapter: 2, span: None };
        assert_eq!((whole.first_api_page(), whole.last_api_page()), (1, None));
        assert_eq!((VerseSource::Juz(30).first_api_page(), VerseSource::Juz(30).last_api_page()), (1, None));
        assert_eq!(VerseSource::Juz(30).next_api_page(1, Some(2)), Some(2));
        assert_eq!(VerseSource::Juz(30).next_api_page(2, Some(2)), None, "an API that repeats a page is not followed");
    }

    #[test]
    fn a_mushaf_page_is_loaded_from_its_listings() {
        // An ordinary page: its own listing, one request.
        let page = VerseSource::Page(3);
        assert_eq!((page.first_api_page(), page.last_api_page()), (1, Some(1)));
        assert_eq!(page.next_api_page(1, None), None);
        assert!(page.loads_at_once() && !VerseSource::Juz(1).loads_at_once());
        // 83:34 is printed on page 589 but listed under 588: two requests, and of
        // the first listing only that verse belongs to the page.
        let turn = VerseSource::Page(589);
        assert_eq!(turn.last_api_page(), Some(2));
        assert_eq!((turn.next_api_page(1, None), turn.next_api_page(2, None)), (Some(2), None));
        assert!(turn.includes(&verse("83:34")) && turn.includes(&verse("84:24")));
        // 83:33 is on page 588; 84:25 is in this page's listing but printed on page 590.
        assert!(!turn.includes(&verse("83:33")) && !turn.includes(&verse("84:25")));
        // Its feed keeps only the page's verses, and is complete after both listings.
        let mut feed = Feed::new(turn);
        feed.merge(1, response(&["83:33", "83:34"], None, 2));
        assert_eq!((feed.verses.len(), feed.next_page, feed.complete()), (1, Some(2), false));
        feed.merge(2, response(&["83:35", "84:25"], None, 2));
        assert_eq!(feed.verses.iter().map(|v| v.verse_key.as_str()).collect::<Vec<_>>(), ["83:34", "83:35"]);
        assert!(feed.complete() && feed.total.is_none());
    }

    #[test]
    fn merges_pages_in_order_and_stops() {
        let mut feed = Feed::new(VerseSource::Juz(1));
        assert_eq!((feed.next_page, feed.complete()), (Some(1), false));
        feed.merge(1, response(&["1:1", "1:2"], Some(2), 4));
        assert_eq!((feed.next_page, feed.total, feed.short_by()), (Some(2), Some(4), None));
        // A page arriving with an earlier verse and a repeat is merged into place.
        feed.merge(2, response(&["1:2", "1:3"], None, 4));
        assert_eq!(feed.verses.iter().map(|v| v.verse_key.as_str()).collect::<Vec<_>>(), ["1:1", "1:2", "1:3"]);
        assert!(feed.complete() && feed.has_verse("1:3") && !feed.has_verse("1:4"));
        assert_eq!(feed.short_by(), Some((3, 4)), "the API counted 4 verses");
        assert!(!feed.merge_issues.is_empty(), "the repeated verse is reported");
    }

    #[test]
    fn a_repeating_or_empty_page_ends_the_feed() {
        let mut feed = Feed::new(VerseSource::Page(3));
        feed.merge(1, response(&["2:6"], Some(1), 1));
        assert!(feed.complete(), "next_page must move forward");
        let mut feed = Feed::new(VerseSource::Page(3));
        feed.merge(1, response(&[], Some(2), 0));
        assert!(feed.complete(), "an empty page has no successor");
    }

    #[test]
    fn range_feed_keeps_only_its_verses_and_pages() {
        let span = VerseSpan { from: 49, to: 52 };
        let mut feed = Feed::new(VerseSource::Chapter { chapter: 2, span: Some(span) });
        assert_eq!(feed.next_page, Some(1));
        feed.merge(1, response(&["2:48", "2:49", "2:50"], Some(2), 286));
        assert_eq!((feed.verses.len(), feed.next_page, feed.total), (2, Some(2), None));
        feed.merge(2, response(&["2:51", "2:52", "2:53"], Some(3), 286));
        assert_eq!((feed.verses.len(), feed.next_page), (4, None));
        assert_eq!(feed.short_by(), None);
    }

    #[test]
    fn expected_verses_are_checked_once_complete() {
        let expected = [(1, VerseSpan { from: 1, to: 3 })];
        let mut feed = Feed::new(VerseSource::Chapter { chapter: 1, span: None });
        feed.merge(1, response(&["1:1", "1:2"], Some(2), 3));
        assert_eq!(feed.issues(&expected), [], "still loading: nothing is missing yet");
        feed.merge(2, response(&[], None, 3));
        let issues = feed.issues(&expected);
        assert_eq!(issues, [IntegrityIssue::MissingVerse { verse_key: "1:3".into() }]);
        let (lines, reload) = issue_messages(&issues);
        assert_eq!(lines, ["Some verses were missing from the server response: 1:3"]);
        assert!(reload);
        assert_eq!(issue_messages(&[]), (Vec::new(), false));
    }
}
