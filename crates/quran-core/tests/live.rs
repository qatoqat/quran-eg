//! Live tests against the real API. Ignored by default (CI/sandbox has no access):
//!
//! ```sh
//! cargo test -p quran-core --test live -- --ignored
//! # or against another deployment:
//! QURAN_API_BASE_URL=https://example.org/api/v4 cargo test -p quran-core --test live -- --ignored
//! ```

use quran_core::api::{Client, DEFAULT_BASE_URL, VerseQuery};

fn client() -> Client {
    Client::new(std::env::var("QURAN_API_BASE_URL").unwrap_or_else(|_| DEFAULT_BASE_URL.into()))
}

#[test]
#[ignore = "hits the real api.quran.com"]
fn live_chapters() {
    let chapters = client().chapters("en").expect("chapters");
    assert_eq!(chapters.len(), 114);
    assert_eq!(chapters[0].name_simple, "Al-Fatihah");
    assert_eq!(chapters[0].verses_count, 7);
    let tawbah = chapters.iter().find(|c| c.id == 9).unwrap();
    assert!(!tawbah.bismillah_pre);
    assert_eq!(
        chapters
            .iter()
            .map(|c| u32::from(c.verses_count))
            .sum::<u32>(),
        6236
    );
}

#[test]
#[ignore = "hits the real api.quran.com"]
fn live_verses_by_chapter_1() {
    let q = VerseQuery {
        // 131 (The Clear Quran) is no longer served; 20 is Saheeh International.
        translations: vec![20],
        ..Default::default()
    };
    let r = client()
        .verses_by_chapter(1, &q)
        .expect("verses_by_chapter");
    assert_eq!(r.verses.len(), 7);
    assert_eq!(r.pagination.total_records, 7);
    assert_eq!(r.pagination.next_page, None);
    let v1 = &r.verses[0];
    assert_eq!(v1.verse_key, "1:1");
    assert!(v1.text_uthmani.as_deref().is_some_and(|t| !t.is_empty()));
    assert!(
        v1.words.last().is_some_and(|w| w.is_end_marker()),
        "{:?}",
        v1.words.last()
    );
    assert!(v1.words.iter().any(|w| {
        w.translation
            .as_ref()
            .and_then(|t| t.text.as_ref())
            .is_some()
    }));
    for v in &r.verses {
        let ids: Vec<u32> = v.translations.iter().map(|t| t.resource_id).collect();
        assert!(ids.contains(&20), "{}: {ids:?}", v.verse_key);
    }
}

#[test]
#[ignore = "hits the real api.quran.com"]
fn live_search_mercy() {
    let s = client().search("mercy", 5, 1, "en").expect("search");
    assert_eq!(s.query, "mercy");
    assert!(s.total_results > 0);
    assert!(!s.results.is_empty() && s.results.len() <= 5);
    assert!(s.results.iter().all(|r| r.verse_key.contains(':')));
}
