//! Live check of the 15-line mushaf layout data the app renders (all 604 pages).
//!
//! Run (needs network, ~604 requests, a few minutes):
//! `cargo test -p quran-core --test live_mushaf -- --ignored --nocapture`
//! Set `QURAN_MUSHAF=1` to check QCF V2 instead of V4 tajweed (19, the default).
//!
//! For every page it asserts: each non-end word has a `code_v2` glyph, its
//! `page_number` (and `v2_page`, when sent) equals the page, and `line_number` is
//! in 1..=15; each end marker has a glyph too. Across all pages every verse
//! appears exactly once and the total is 6,236.

use std::collections::HashMap;
use std::time::Duration;

use quran_core::api::{Client, DEFAULT_BASE_URL, VerseQuery, VersesResponse};

const PAGES: u16 = 604;
const TOTAL_VERSES: usize = 6236;

fn query(page: u32, mushaf: u32) -> VerseQuery {
    VerseQuery {
        language: "en".into(),
        translations: vec![],
        words: true,
        fields: vec![],
        word_fields: ["location", "code_v2", "v2_page", "page_number", "line_number", "qpc_uthmani_hafs"]
            .iter()
            .map(|s| s.to_string())
            .collect(),
        translation_fields: vec![],
        page,
        per_page: 50,
        mushaf: Some(mushaf),
    }
}

fn fetch(c: &Client, page: u16, api_page: u32, mushaf: u32) -> Result<VersesResponse, String> {
    let mut last = String::new();
    for attempt in 1..=3u64 {
        std::thread::sleep(Duration::from_millis(100 * attempt));
        match c.verses_by_page(page, &query(api_page, mushaf)) {
            Ok(r) => return Ok(r),
            Err(e) => last = e.to_string(),
        }
    }
    Err(last)
}

#[test]
#[ignore = "hits the real api.quran.com (604 pages)"]
fn live_all_pages_have_complete_15_line_layout() {
    let c = Client::new(std::env::var("QURAN_API_BASE_URL").unwrap_or_else(|_| DEFAULT_BASE_URL.into()));
    let mushaf: u32 = std::env::var("QURAN_MUSHAF").ok().and_then(|s| s.parse().ok()).unwrap_or(19);
    let mut problems: Vec<String> = Vec::new();
    let mut seen: HashMap<String, u16> = HashMap::new();

    for page in 1..=PAGES {
        let mut api_page = 1;
        loop {
            let r = match fetch(&c, page, api_page, mushaf) {
                Ok(r) => r,
                Err(e) => {
                    problems.push(format!("page {page}: request failed: {e}"));
                    break;
                }
            };
            for v in &r.verses {
                if let Some(prev) = seen.insert(v.verse_key.clone(), page) {
                    problems.push(format!("{}: on page {prev} and again on page {page}", v.verse_key));
                }
                for w in &v.words {
                    let at = format!("page {page} {}:{}", v.verse_key, w.position);
                    if w.code_v2.as_deref().is_none_or(|g| g.trim().is_empty()) {
                        problems.push(format!("{at}: no code_v2 glyph"));
                    }
                    if w.is_end_marker() {
                        continue;
                    }
                    if w.page_number != Some(page) {
                        problems.push(format!("{at}: page_number {:?}", w.page_number));
                    }
                    if let Some(p2) = w.v2_page
                        && p2 != page
                    {
                        problems.push(format!("{at}: v2_page {p2} (font file differs from page)"));
                    }
                    match w.line_number {
                        Some(1..=15) => {}
                        other => problems.push(format!("{at}: line_number {other:?}")),
                    }
                }
            }
            match r.pagination.next_page {
                Some(n) if n > api_page => api_page = n,
                _ => break,
            }
        }
        if page % 50 == 0 {
            eprintln!("checked {page}/{PAGES} pages, {} verses, {} problems", seen.len(), problems.len());
        }
    }
    if seen.len() != TOTAL_VERSES {
        problems.push(format!("{} distinct verses across all pages, expected {TOTAL_VERSES}", seen.len()));
    }
    for p in problems.iter().take(200) {
        eprintln!("  {p}");
    }
    assert!(problems.is_empty(), "{} mushaf layout problem(s); first ones printed above", problems.len());
}
