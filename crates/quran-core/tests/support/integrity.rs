//! Pure (network-free) integrity checks for verse/word data returned by the API.
//!
//! Shared by `tests/live_integrity.rs` (runs them over the whole Quran) and
//! `tests/integrity.rs` (unit-tests them offline against fixtures). Every check
//! returns human-readable problems prefixed with the verse key instead of failing
//! fast, so a single live run reports *all* gaps.

use quran_core::api::{Verse, Word};

/// Total number of verses in the Quran (Hafs, Kufan count).
pub const TOTAL_VERSES: usize = 6236;

/// Quranic annotation signs that appear as stand-alone whitespace-separated
/// tokens in verse-level `text_uthmani` but are not words, plus digits used for
/// ayah numbers:
/// - U+06D6..=U+06DC  small high waqf (pause) signs, e.g. `ۚ` `ۖ` `ۗ`
/// - U+06DD           end of ayah
/// - U+06DE           rub el hizb `۞`
/// - U+06E9           sajdah `۩`
/// - Arabic-Indic / extended Arabic-Indic / ASCII digits
pub fn is_annotation_mark(c: char) -> bool {
    matches!(c,
        '\u{06D6}'..='\u{06DE}'
        | '\u{06E9}'
        | '\u{0660}'..='\u{0669}'
        | '\u{06F0}'..='\u{06F9}'
        | '0'..='9')
}

/// Whitespace-separated word tokens of `text`, skipping tokens made only of
/// annotation marks (pause/sajdah/hizb signs and ayah numbers).
pub fn word_tokens(text: &str) -> Vec<&str> {
    text.split_whitespace()
        .filter(|t| !t.chars().all(is_annotation_mark))
        .collect()
}

/// Words that are actual Quranic words (not the `end` ayah-number marker, nor
/// pause/sajdah/hizb glyph entries if the API ever emits them).
fn is_real_word(w: &Word) -> bool {
    w.char_type_name == "word"
}

/// Check one verse's words against itself and its verse-level `text_uthmani`.
pub fn check_verse(v: &Verse) -> Vec<String> {
    let key = &v.verse_key;
    let mut problems = Vec::new();

    if v.chapter_number().is_none() || !key.ends_with(&format!(":{}", v.verse_number)) {
        problems.push(format!(
            "{key}: verse_key does not match verse_number {}",
            v.verse_number
        ));
    }
    if v.words.is_empty() {
        problems.push(format!("{key}: no words returned"));
        return problems;
    }

    // Positions must be exactly 1, 2, 3, ... in order.
    for (i, w) in v.words.iter().enumerate() {
        let expected = i + 1;
        if usize::from(w.position) != expected {
            problems.push(format!(
                "{key}: word positions not contiguous: index {i} has position {} (expected {expected})",
                w.position
            ));
            break;
        }
    }

    // Exactly one end marker, and it is the last word.
    let ends: Vec<usize> = v
        .words
        .iter()
        .enumerate()
        .filter(|(_, w)| w.is_end_marker())
        .map(|(i, _)| i)
        .collect();
    match ends.as_slice() {
        [] => problems.push(format!("{key}: missing `end` marker word")),
        [i] if *i != v.words.len() - 1 => problems.push(format!(
            "{key}: `end` marker is at position {} but is not the last word",
            v.words[*i].position
        )),
        [_] => {}
        many => problems.push(format!(
            "{key}: {} `end` marker words (expected 1)",
            many.len()
        )),
    }

    // Every non-end word must have visible Uthmani text.
    for w in v.words.iter().filter(|w| !w.is_end_marker()) {
        if w.text_uthmani
            .as_deref()
            .is_none_or(|t| t.trim().is_empty())
        {
            problems.push(format!(
                "{key}: word at position {} has empty text_uthmani",
                w.position
            ));
        }
    }

    // Word count must match the verse text, and so must the words themselves.
    match v.text_uthmani.as_deref() {
        None | Some("") => problems.push(format!("{key}: verse has no text_uthmani")),
        Some(text) => {
            let verse_tokens = word_tokens(text);
            let words: Vec<&Word> = v.words.iter().filter(|w| is_real_word(w)).collect();
            if words.len() != verse_tokens.len() {
                problems.push(format!(
                    "{key}: {} words but verse text_uthmani has {} tokens",
                    words.len(),
                    verse_tokens.len()
                ));
            } else {
                let word_tokens: Vec<&str> = words
                    .iter()
                    .flat_map(|w| word_tokens(w.text_uthmani.as_deref().unwrap_or("")))
                    .collect();
                if let Some(i) =
                    (0..verse_tokens.len()).find(|&i| word_tokens.get(i) != Some(&verse_tokens[i]))
                {
                    problems.push(format!(
                        "{key}: word {} is \"{}\" but verse text has \"{}\"",
                        i + 1,
                        word_tokens.get(i).copied().unwrap_or(""),
                        verse_tokens[i]
                    ));
                }
            }
        }
    }

    problems
}

/// Check that `verses` are exactly verses `1..=verses_count` of `chapter`, in
/// order, and run [`check_verse`] on each.
pub fn check_chapter(chapter: u16, verses_count: u16, verses: &[Verse]) -> Vec<String> {
    let mut problems = Vec::new();
    let numbers: Vec<u16> = verses.iter().map(|v| v.verse_number).collect();
    let expected: Vec<u16> = (1..=verses_count).collect();

    if numbers != expected {
        let missing: Vec<u16> = expected
            .iter()
            .copied()
            .filter(|n| !numbers.contains(n))
            .collect();
        let mut seen = std::collections::BTreeSet::new();
        let unexpected: Vec<u16> = numbers
            .iter()
            .copied()
            .filter(|n| *n == 0 || *n > verses_count || !seen.insert(*n))
            .collect();
        let detail = if missing.is_empty() && unexpected.is_empty() {
            "out of order".to_string()
        } else {
            format!("missing {missing:?}, unexpected/duplicate {unexpected:?}")
        };
        problems.push(format!(
            "chapter {chapter}: got {} verses, expected {verses_count}: {detail}",
            numbers.len()
        ));
    }

    for v in verses {
        if v.chapter_number() != Some(chapter) {
            problems.push(format!("{}: returned for chapter {chapter}", v.verse_key));
        }
        problems.extend(check_verse(v));
    }
    problems
}

/// Words with no Indo-Pak text. Not fatal (only matters for the Indo-Pak
/// script setting), reported as warnings.
pub fn indopak_warnings(v: &Verse) -> Vec<String> {
    v.words
        .iter()
        .filter(|w| {
            !w.is_end_marker()
                && w.text_indopak
                    .as_deref()
                    .is_none_or(|t| t.trim().is_empty())
        })
        .map(|w| {
            format!(
                "{}: word at position {} has no text_indopak",
                v.verse_key, w.position
            )
        })
        .collect()
}
