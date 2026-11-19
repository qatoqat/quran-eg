//! Whole-Quran integrity check against the live API: verifies that no verse or
//! word is missing from the data the app renders. Ignored by default because it
//! makes ~190 sequential requests (one per 50-verse page, 100 ms apart).
//!
//! ```sh
//! cargo test -p quran-core --test live_integrity -- --ignored --nocapture
//! # against another deployment:
//! QURAN_API_BASE_URL=https://example.org/api/v4 \
//!     cargo test -p quran-core --test live_integrity -- --ignored --nocapture
//! ```
//!
//! For every chapter it pages through `/verses/by_chapter` and checks (see
//! `tests/support/integrity.rs`, unit-tested offline in `tests/integrity.rs`):
//! - 114 chapters, 6236 verses in total;
//! - each chapter's verse numbers are exactly `1..=verses_count`;
//! - per verse: word positions contiguous from 1, exactly one `end` word and it
//!   is last, every non-end word has non-empty `text_uthmani`, and the words
//!   match the whitespace-separated tokens of the verse's `text_uthmani`
//!   (stand-alone pause/sajdah/hizb signs are not counted as words).
//!
//! All problems are collected and printed with their verse keys before the
//! final assertion. Missing Indo-Pak word text is printed as a warning only.

#[allow(dead_code)]
mod support;

use quran_core::api::{Client, DEFAULT_BASE_URL, Verse, VerseQuery, VersesResponse};
use std::time::Duration;
use support::integrity::*;

const DELAY: Duration = Duration::from_millis(100);
const MAX_ATTEMPTS: u32 = 3;

fn client() -> Client {
    Client::new(std::env::var("QURAN_API_BASE_URL").unwrap_or_else(|_| DEFAULT_BASE_URL.into()))
}

fn query(page: u32) -> VerseQuery {
    VerseQuery {
        language: "en".into(),
        translations: vec![],
        words: true,
        fields: vec!["text_uthmani".into()],
        word_fields: vec!["text_uthmani".into(), "text_indopak".into()],
        translation_fields: vec![],
        page,
        per_page: 50,
        mushaf: None,
    }
}

/// Fetch one page politely: delay before every request, retry with backoff.
fn fetch_page(c: &Client, chapter: u16, page: u32) -> Result<VersesResponse, String> {
    let mut last_err = String::new();
    for attempt in 1..=MAX_ATTEMPTS {
        std::thread::sleep(DELAY * attempt * attempt);
        match c.verses_by_chapter(chapter, &query(page)) {
            Ok(r) => return Ok(r),
            Err(e) => last_err = format!("attempt {attempt}: {e}"),
        }
    }
    Err(format!("chapter {chapter} page {page}: {last_err}"))
}

fn fetch_chapter(c: &Client, chapter: u16) -> Result<Vec<Verse>, String> {
    let mut verses = Vec::new();
    let mut page = 1;
    loop {
        let r = fetch_page(c, chapter, page)?;
        verses.extend(r.verses);
        match r.pagination.next_page {
            Some(next) if next > page && next <= 20 => page = next,
            Some(next) => {
                return Err(format!(
                    "chapter {chapter}: bad next_page {next} after {page}"
                ));
            }
            None => return Ok(verses),
        }
    }
}

#[test]
#[ignore = "hits the real api.quran.com (~190 requests)"]
fn every_verse_and_word_is_present() {
    let c = client();
    let chapters = c.chapters("en").expect("chapters");

    let mut problems = Vec::new();
    let mut warnings = Vec::new();
    if chapters.len() != 114 {
        problems.push(format!("expected 114 chapters, got {}", chapters.len()));
    }

    let mut total_verses = 0usize;
    let mut total_words = 0usize;
    for ch in &chapters {
        match fetch_chapter(&c, ch.id) {
            Ok(verses) => {
                total_verses += verses.len();
                total_words += verses
                    .iter()
                    .map(|v| v.words.iter().filter(|w| !w.is_end_marker()).count())
                    .sum::<usize>();
                let found = check_chapter(ch.id, ch.verses_count, &verses);
                warnings.extend(verses.iter().flat_map(indopak_warnings));
                println!(
                    "chapter {:>3} {:<20} {:>3} verses  {}",
                    ch.id,
                    ch.name_simple,
                    verses.len(),
                    if found.is_empty() {
                        "ok".to_string()
                    } else {
                        format!("{} problem(s)", found.len())
                    }
                );
                problems.extend(found);
            }
            Err(e) => problems.push(e),
        }
    }

    if total_verses != TOTAL_VERSES {
        problems.push(format!(
            "expected {TOTAL_VERSES} verses in total, got {total_verses}"
        ));
    }

    println!("\n{total_verses} verses, {total_words} words checked");
    if !warnings.is_empty() {
        println!("\n{} warning(s):", warnings.len());
        for w in warnings.iter().take(200) {
            println!("  {w}");
        }
    }
    if !problems.is_empty() {
        println!("\n{} PROBLEM(S):", problems.len());
        for p in &problems {
            println!("  {p}");
        }
    }
    assert!(
        problems.is_empty(),
        "{} integrity problem(s); run with --nocapture to list them",
        problems.len()
    );
}
