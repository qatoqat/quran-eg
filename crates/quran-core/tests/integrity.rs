//! Offline unit tests for the integrity checker used by `tests/live_integrity.rs`.

#[allow(dead_code)]
mod support;

use pretty_assertions::assert_eq;
use quran_core::api::{SearchResponse, Verse, VerseResponse, VersesResponse};
use support::integrity::*;

fn fixture<T: serde::de::DeserializeOwned>(name: &str) -> T {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn verse(json: &str) -> Verse {
    serde_json::from_str(json).unwrap()
}

#[test]
fn good_fixture_has_no_problems() {
    let r: VersesResponse = fixture("verses_by_chapter.json");
    for v in &r.verses {
        assert_eq!(check_verse(v), Vec::<String>::new(), "{}", v.verse_key);
    }
    // The fixture only holds the first page (1:1-1:3) of a 3-verse "chapter".
    assert_eq!(check_chapter(1, 3, &r.verses), Vec::<String>::new());
    assert!(
        r.verses
            .iter()
            .all(|v| indopak_warnings(v).len() == v.words.len() - 1)
    );
}

#[test]
fn partial_chapter_reports_missing_verses() {
    let r: VersesResponse = fixture("verses_by_chapter.json");
    assert_eq!(
        check_chapter(1, 7, &r.verses),
        vec!["chapter 1: got 3 verses, expected 7: missing [4, 5, 6, 7], unexpected/duplicate []"]
    );
}

#[test]
fn broken_fixture_reports_every_problem() {
    let r: VersesResponse = fixture("broken_verses_by_chapter.json");
    let problems = check_chapter(1, 3, &r.verses);
    for p in &problems {
        println!("{p}");
    }
    assert_eq!(
        problems,
        vec![
            // Gap in verse numbers: 1, 2, 4.
            "chapter 1: got 3 verses, expected 3: missing [3], unexpected/duplicate [4]",
            // 1:1 lost its 3rd word.
            "1:1: word positions not contiguous: index 2 has position 4 (expected 3)",
            "1:1: 3 words but verse text_uthmani has 4 tokens",
            // 1:2 lost its end marker.
            "1:2: missing `end` marker word",
            // 1:4: end marker first, one word with empty text.
            "1:4: `end` marker is at position 1 but is not the last word",
            "1:4: word at position 3 has empty text_uthmani",
            "1:4: word 2 is \"\" but verse text has \"ٱلرَّحِيمِ\"",
        ]
    );
}

#[test]
fn missing_position_field_fails_to_deserialize() {
    // A word without `position` is rejected at parse time, so the live test fails loudly.
    let err = serde_json::from_str::<quran_core::api::Word>(r#"{"id": 1, "char_type_name": "word"}"#)
        .unwrap_err();
    assert!(err.to_string().contains("position"), "{err}");
}

#[test]
fn duplicate_end_markers_and_empty_words_are_reported() {
    let v = verse(
        r#"{"id": 1, "verse_number": 1, "verse_key": "1:1", "text_uthmani": "ٱلْحَمْدُ",
            "words": [
              {"id": 1, "position": 1, "char_type_name": "end", "text_uthmani": "١"},
              {"id": 2, "position": 2, "char_type_name": "word", "text_uthmani": "ٱلْحَمْدُ"},
              {"id": 3, "position": 3, "char_type_name": "end", "text_uthmani": "١"}
            ]}"#,
    );
    assert_eq!(
        check_verse(&v),
        vec!["1:1: 2 `end` marker words (expected 1)"]
    );

    let v = verse(r#"{"id": 1, "verse_number": 1, "verse_key": "1:1", "text_uthmani": "x"}"#);
    assert_eq!(check_verse(&v), vec!["1:1: no words returned"]);

    let v = verse(
        r#"{"id": 1, "verse_number": 2, "verse_key": "1:1",
            "words": [{"id": 1, "position": 1, "char_type_name": "end"}]}"#,
    );
    assert_eq!(
        check_verse(&v),
        vec![
            "1:1: verse_key does not match verse_number 2",
            "1:1: verse has no text_uthmani"
        ]
    );
}

#[test]
fn swapped_words_are_reported_even_when_counts_match() {
    let v = verse(
        r#"{"id": 3, "verse_number": 3, "verse_key": "1:3", "text_uthmani": "ٱلرَّحْمَٰنِ ٱلرَّحِيمِ",
            "words": [
              {"id": 11, "position": 1, "char_type_name": "word", "text_uthmani": "ٱلرَّحِيمِ"},
              {"id": 12, "position": 2, "char_type_name": "word", "text_uthmani": "ٱلرَّحْمَٰنِ"},
              {"id": 13, "position": 3, "char_type_name": "end", "text_uthmani": "٣"}
            ]}"#,
    );
    assert_eq!(
        check_verse(&v),
        vec!["1:3: word 1 is \"ٱلرَّحِيمِ\" but verse text has \"ٱلرَّحْمَٰنِ\""]
    );
}

#[test]
fn pause_marks_are_not_words() {
    // Ayat al-Kursi: 58 whitespace tokens, 8 of them stand-alone pause marks, 50 words.
    let r: VerseResponse = fixture("verse_by_key.json");
    let text = r.verse.text_uthmani.as_deref().unwrap();
    assert_eq!(text.split_whitespace().count(), 58);
    assert_eq!(word_tokens(text).len(), 50);

    // Rub el hizb sign at the start of 39:53 is not a word either.
    let s: SearchResponse = fixture("search.json");
    let zumar = s
        .search
        .results
        .iter()
        .find(|r| r.verse_key == "39:53")
        .unwrap();
    let tokens = word_tokens(&zumar.text);
    assert_eq!(tokens[0], "قُلْ");
    assert_eq!(tokens.len(), 20);

    // A pause mark attached to a word's own text (with a space) is tolerated.
    let v = verse(
        r#"{"id": 9, "verse_number": 1, "verse_key": "2:1", "text_uthmani": "ٱلْحَىُّ ٱلْقَيُّومُ ۚ لَا",
            "words": [
              {"id": 1, "position": 1, "char_type_name": "word", "text_uthmani": "ٱلْحَىُّ"},
              {"id": 2, "position": 2, "char_type_name": "word", "text_uthmani": "ٱلْقَيُّومُ ۚ"},
              {"id": 3, "position": 3, "char_type_name": "word", "text_uthmani": "لَا"},
              {"id": 4, "position": 4, "char_type_name": "end", "text_uthmani": "١"}
            ]}"#,
    );
    assert_eq!(check_verse(&v), Vec::<String>::new());
}
