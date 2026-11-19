//! Pure integrity checks over loaded verses, so no word or verse of the Quran
//! silently goes missing from the view. Nothing here panics; every problem is
//! reported as an [`IntegrityIssue`].
//!
//! - (a) [`sort_dedup`]: verses sorted by (chapter, verse) and deduplicated by key.
//! - (b) [`check_gaps`] / [`check_expected`]: verse numbers contiguous.
//! - (c) + (d) [`word_issues`]: per-verse word structure and per-word text.
//! - (f) [`reference_issues`]: each verse exists and has the number of words the
//!   compiled-in [`reference`] tables say it has.
//! - (g) [`line_issues`]: each mushaf line holds as many words as it is printed with.
//! - (e) [`layout_issues`]: 15-line mushaf layout data (`page_number`/`line_number`
//!   on every word, lines 1..=15, at most 15 lines per page, lines in order, glyph
//!   font page = layout page).

use std::fmt;

use crate::api::{Verse, Word};

use crate::parse::VerseSpan;
use crate::reference;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntegrityIssue {
    /// The same verse arrived more than once (dropped).
    Duplicate { verse_key: String },
    /// Verses arrived out of mushaf order (re-sorted).
    OutOfOrder { verse_key: String },
    /// A verse that should be shown is not in the list.
    MissingVerse { verse_key: String },
    /// A verse came without any words.
    NoWords { verse_key: String },
    /// Word positions are not 1, 2, 3, …: `position` is the expected one.
    WordPositionGap { verse_key: String, position: u16 },
    /// No `end` (ayah number) marker.
    MissingEndMarker { verse_key: String },
    /// More than one `end` marker; `position` is the extra one.
    ExtraEndMarker { verse_key: String, position: u16 },
    /// The `end` marker is not the last word.
    EndMarkerNotLast { verse_key: String, position: u16 },
    /// A word has no text for the selected script nor any fallback.
    EmptyWordText { verse_key: String, position: u16 },
    /// The verse has `found` words but is known to have `expected`.
    WordCount { verse_key: String, expected: usize, found: usize },
    /// A verse number beyond the end of its surah.
    UnknownVerse { verse_key: String },
    /// A mushaf line shows `found` words (ayah markers included) but is known
    /// to hold `expected`; `verse_key` is a verse on it, or the page's first.
    LineWordCount { verse_key: String, page: u16, line: u16, expected: usize, found: usize },
    /// A word has no `page_number`/`line_number`: the verse is shown as flowing text.
    MissingLayout { verse_key: String, position: u16 },
    /// A word's `line_number` is outside 1..=15: the verse is shown as flowing text.
    LineOutOfRange { verse_key: String, position: u16, line: u16 },
    /// A page would have more than 15 lines (`verse_key` starts the extra line).
    TooManyLines { verse_key: String, page: u16 },
    /// Lines or pages go backwards (`verse_key` is where it happens).
    LayoutOutOfOrder { verse_key: String, page: u16 },
    /// `v2_page` differs from `page_number`: the word is drawn as Unicode text.
    GlyphPageMismatch { verse_key: String, position: u16 },
}

impl IntegrityIssue {
    pub fn verse_key(&self) -> &str {
        match self {
            IntegrityIssue::Duplicate { verse_key }
            | IntegrityIssue::OutOfOrder { verse_key }
            | IntegrityIssue::MissingVerse { verse_key }
            | IntegrityIssue::NoWords { verse_key }
            | IntegrityIssue::WordPositionGap { verse_key, .. }
            | IntegrityIssue::MissingEndMarker { verse_key }
            | IntegrityIssue::ExtraEndMarker { verse_key, .. }
            | IntegrityIssue::EndMarkerNotLast { verse_key, .. }
            | IntegrityIssue::WordCount { verse_key, .. }
            | IntegrityIssue::UnknownVerse { verse_key }
            | IntegrityIssue::LineWordCount { verse_key, .. }
            | IntegrityIssue::EmptyWordText { verse_key, .. }
            | IntegrityIssue::MissingLayout { verse_key, .. }
            | IntegrityIssue::LineOutOfRange { verse_key, .. }
            | IntegrityIssue::TooManyLines { verse_key, .. }
            | IntegrityIssue::LayoutOutOfOrder { verse_key, .. }
            | IntegrityIssue::GlyphPageMismatch { verse_key, .. } => verse_key,
        }
    }

    /// Word-level problems: the verse is rendered from verse-level text instead.
    pub fn is_word_issue(&self) -> bool {
        matches!(
            self,
            IntegrityIssue::NoWords { .. }
                | IntegrityIssue::WordPositionGap { .. }
                | IntegrityIssue::MissingEndMarker { .. }
                | IntegrityIssue::ExtraEndMarker { .. }
                | IntegrityIssue::EndMarkerNotLast { .. }
                | IntegrityIssue::WordCount { .. }
                | IntegrityIssue::EmptyWordText { .. }
        )
    }

    /// Word problems that make the verse fall back to its verse-level text
    /// (every word issue except a single word without text).
    pub fn is_structural_word_issue(&self) -> bool {
        self.is_word_issue() && !matches!(self, IntegrityIssue::EmptyWordText { .. })
    }

    /// Missing/invalid line data: the verse is rendered as flowing text.
    pub fn is_layout_issue(&self) -> bool {
        matches!(self, IntegrityIssue::MissingLayout { .. } | IntegrityIssue::LineOutOfRange { .. })
    }

    /// The page layout looks inconsistent, but every word is still shown.
    pub fn is_page_shape_issue(&self) -> bool {
        matches!(self, IntegrityIssue::TooManyLines { .. } | IntegrityIssue::LayoutOutOfOrder { .. })
    }
}

impl fmt::Display for IntegrityIssue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IntegrityIssue::Duplicate { verse_key } => write!(f, "{verse_key}: duplicate verse"),
            IntegrityIssue::OutOfOrder { verse_key } => write!(f, "{verse_key}: out of order"),
            IntegrityIssue::MissingVerse { verse_key } => write!(f, "{verse_key}: verse missing"),
            IntegrityIssue::NoWords { verse_key } => write!(f, "{verse_key}: no words"),
            IntegrityIssue::WordPositionGap { verse_key, position } => {
                write!(f, "{verse_key}: word position {position} missing")
            }
            IntegrityIssue::MissingEndMarker { verse_key } => {
                write!(f, "{verse_key}: no end-of-verse marker")
            }
            IntegrityIssue::ExtraEndMarker { verse_key, position } => {
                write!(f, "{verse_key}: extra end marker at word {position}")
            }
            IntegrityIssue::EndMarkerNotLast { verse_key, position } => {
                write!(f, "{verse_key}: end marker at word {position} is not last")
            }
            IntegrityIssue::EmptyWordText { verse_key, position } => {
                write!(f, "{verse_key}: word {position} has no text")
            }
            IntegrityIssue::WordCount { verse_key, expected, found } => {
                write!(f, "{verse_key}: {found} words instead of {expected}")
            }
            IntegrityIssue::UnknownVerse { verse_key } => write!(f, "{verse_key}: no such verse"),
            IntegrityIssue::LineWordCount { page, line, expected, found, .. } => {
                write!(f, "page {page} line {line}: {found} of {expected} words")
            }
            IntegrityIssue::MissingLayout { verse_key, position } => {
                write!(f, "{verse_key}: word {position} has no mushaf page/line")
            }
            IntegrityIssue::LineOutOfRange { verse_key, position, line } => {
                write!(f, "{verse_key}: word {position} is on line {line} (expected 1-{MUSHAF_LINES})")
            }
            IntegrityIssue::TooManyLines { verse_key, page } => {
                write!(f, "{verse_key}: page {page} has more than {MUSHAF_LINES} lines")
            }
            IntegrityIssue::LayoutOutOfOrder { verse_key, page } => {
                write!(f, "{verse_key}: mushaf lines out of order on page {page}")
            }
            IntegrityIssue::GlyphPageMismatch { verse_key, position } => {
                write!(f, "{verse_key}: word {position} glyph page differs from its layout page")
            }
        }
    }
}

/// `(chapter, verse)` sort key; malformed keys sort last.
fn sort_key(v: &Verse) -> (u16, u16) {
    (v.chapter_number().unwrap_or(u16::MAX), v.verse_number)
}

/// (a) Sort verses into mushaf order and drop duplicate `verse_key`s (keeping
/// the first copy). Reports one `OutOfOrder` (first offender) and every duplicate.
pub fn sort_dedup(verses: &mut Vec<Verse>) -> Vec<IntegrityIssue> {
    let mut issues = Vec::new();
    if let Some(w) = verses.windows(2).find(|w| sort_key(&w[0]) > sort_key(&w[1])) {
        issues.push(IntegrityIssue::OutOfOrder { verse_key: w[1].verse_key.clone() });
    }
    // Stable sort keeps the first-arrived copy of a duplicate first.
    verses.sort_by_key(sort_key);
    let mut seen = std::collections::HashSet::new();
    verses.retain(|v| {
        let fresh = seen.insert(v.verse_key.clone());
        if !fresh {
            issues.push(IntegrityIssue::Duplicate { verse_key: v.verse_key.clone() });
        }
        fresh
    });
    issues
}

/// (b) Gaps between consecutive verses of the same chapter (list must be sorted).
pub fn check_gaps(verses: &[Verse]) -> Vec<IntegrityIssue> {
    let mut issues = Vec::new();
    for w in verses.windows(2) {
        let (a, b) = (sort_key(&w[0]), sort_key(&w[1]));
        if a.0 == b.0 && b.1 > a.1.saturating_add(1) {
            for n in a.1 + 1..b.1 {
                issues.push(IntegrityIssue::MissingVerse { verse_key: format!("{}:{n}", a.0) });
            }
        }
    }
    issues
}

/// (b) Every verse of `expected` in `chapter` must be present. Call this only
/// once all pages are loaded (otherwise the tail is merely not loaded yet).
pub fn check_expected(verses: &[Verse], chapter: u16, expected: VerseSpan) -> Vec<IntegrityIssue> {
    let present: std::collections::HashSet<u16> =
        verses.iter().filter(|v| v.chapter_number() == Some(chapter)).map(|v| v.verse_number).collect();
    (expected.from..=expected.to)
        .filter(|n| !present.contains(n))
        .map(|n| IntegrityIssue::MissingVerse { verse_key: format!("{chapter}:{n}") })
        .collect()
}

/// Text of `word` for the script whose API field is `script_field`
/// (`QuranScript::field()`), falling back to `qpc_uthmani_hafs`, `text_uthmani`, then `text`.
pub fn word_text<'a>(word: &'a Word, script_field: &str) -> Option<&'a str> {
    let selected = match script_field {
        "text_indopak" => word.text_indopak.as_deref(),
        "text_imlaei" => word.text_imlaei.as_deref(),
        "qpc_uthmani_hafs" => word.qpc_uthmani_hafs.as_deref(),
        _ => word.text_uthmani.as_deref(),
    };
    [selected, word.qpc_uthmani_hafs.as_deref(), word.text_uthmani.as_deref(), word.text.as_deref()]
        .into_iter()
        .flatten()
        .find(|t| !t.trim().is_empty())
}

/// Verse-level text for the selected script, falling back to the other scripts.
pub fn verse_text<'a>(verse: &'a Verse, script_field: &str) -> Option<&'a str> {
    let selected = match script_field {
        "text_indopak" => verse.text_indopak.as_deref(),
        "text_imlaei" => verse.text_imlaei.as_deref(),
        _ => verse.text_uthmani.as_deref(),
    };
    [selected, verse.text_uthmani.as_deref(), verse.text_indopak.as_deref(), verse.text_imlaei.as_deref()]
        .into_iter()
        .flatten()
        .find(|t| !t.trim().is_empty())
}

/// (c) + (d) Word structure and text of one verse.
pub fn word_issues(verse: &Verse, script_field: &str) -> Vec<IntegrityIssue> {
    let key = || verse.verse_key.clone();
    let words = &verse.words;
    if words.is_empty() {
        return vec![IntegrityIssue::NoWords { verse_key: key() }];
    }
    let mut issues = Vec::new();
    // Positions: the API sends words in order, 1-based and contiguous.
    let mut expected: u16 = 1;
    for w in words {
        if w.position != expected {
            issues.push(IntegrityIssue::WordPositionGap { verse_key: key(), position: expected });
            break;
        }
        expected = expected.saturating_add(1);
    }
    let ends: Vec<&Word> = words.iter().filter(|w| w.is_end_marker()).collect();
    match ends.as_slice() {
        [] => issues.push(IntegrityIssue::MissingEndMarker { verse_key: key() }),
        [first, rest @ ..] => {
            for extra in rest {
                issues.push(IntegrityIssue::ExtraEndMarker { verse_key: key(), position: extra.position });
            }
            if rest.is_empty() && !words.last().is_some_and(|w| w.is_end_marker()) {
                issues.push(IntegrityIssue::EndMarkerNotLast { verse_key: key(), position: first.position });
            }
        }
    }
    for w in words.iter().filter(|w| !w.is_end_marker()) {
        if word_text(w, script_field).is_none() {
            issues.push(IntegrityIssue::EmptyWordText { verse_key: key(), position: w.position });
        }
    }
    issues
}

#[cfg(test)]
thread_local! {
    /// Unit tests build verses with made-up word counts, so there the reference
    /// check runs only for the tests that turn it on.
    pub(crate) static CHECK_REFERENCE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// (f) A verse against the compiled-in reference: it must exist, and have the
/// number of words it is known to have. This catches a response that lost (or
/// gained) a word although its word positions are contiguous.
pub fn reference_issues(verse: &Verse) -> Vec<IntegrityIssue> {
    #[cfg(test)]
    if !CHECK_REFERENCE.get() {
        return Vec::new();
    }
    let Some(chapter) = verse.chapter_number() else { return Vec::new() };
    let found = word_count(verse);
    match reference::words_in(chapter, verse.verse_number) {
        None => vec![IntegrityIssue::UnknownVerse { verse_key: verse.verse_key.clone() }],
        // A verse without any words is reported as such by `word_issues`.
        Some(expected) if expected != found && !verse.words.is_empty() => {
            vec![IntegrityIssue::WordCount { verse_key: verse.verse_key.clone(), expected, found }]
        }
        Some(_) => Vec::new(),
    }
}

/// (g) Lines against the compiled-in reference: a mushaf line must hold as many
/// words as it is printed with. This is what notices a line that is only
/// partly there, for example at a page turn where the API lists a verse on the
/// other side.
///
/// `whole_page`: the view is that one mushaf page, so all of its lines must be
/// there and complete, and words on other pages are not shown. Otherwise the
/// first and the last line of what is loaded may be cut (a range, or more still
/// to load) and only the lines between them are checked. Call it for a whole
/// page only once everything has loaded.
pub fn line_issues(verses: &[Verse], whole_page: Option<u16>) -> Vec<IntegrityIssue> {
    #[cfg(test)]
    if !CHECK_REFERENCE.get() {
        return Vec::new();
    }
    // Verses shown as flowing text are not on their lines; other checks report them.
    if verses.iter().any(|v| !words_structurally_ok(v) || !verse_layout_ok(v)) {
        return Vec::new();
    }
    // Lines in reading order: ((page, line), words on it, a verse on it).
    let mut lines: Vec<((u16, u16), usize, &str)> = Vec::new();
    for v in verses {
        let mut prev = None;
        for w in &v.words {
            let Some(slot) = slot_after(prev, w) else { continue };
            prev = Some(slot);
            if whole_page.is_some_and(|p| p != slot.0) {
                continue;
            }
            match lines.last_mut() {
                Some((last, count, _)) if *last == slot => *count += 1,
                _ => lines.push((slot, 1, &v.verse_key)),
            }
        }
    }
    let issue = |(page, line): (u16, u16), found: usize, verse_key: &str| {
        let expected = reference::words_on_line(page, line)?;
        (expected != found).then(|| IntegrityIssue::LineWordCount {
            verse_key: verse_key.to_string(),
            page,
            line,
            expected,
            found,
        })
    };
    match whole_page {
        Some(page) => {
            let first_key = lines.first().map_or("", |l| l.2);
            (1..=MUSHAF_LINES)
                .filter_map(|line| match lines.iter().find(|l| l.0 == (page, line)) {
                    Some(&(slot, found, key)) => issue(slot, found, key),
                    None => issue((page, line), 0, first_key),
                })
                .collect()
        }
        None if lines.len() > 2 => {
            lines[1..lines.len() - 1].iter().filter_map(|&(slot, found, key)| issue(slot, found, key)).collect()
        }
        None => Vec::new(),
    }
}

/// Number of non-end-marker words.
pub fn word_count(verse: &Verse) -> usize {
    verse.words.iter().filter(|w| !w.is_end_marker()).count()
}

/// (b) Gaps across chapter boundaries (list must be sorted). When consecutive
/// verses belong to different chapters, the earlier chapter must end at its last
/// verse, every chapter in between must be fully present, and the later chapter
/// must start at verse 1. `verses_count` gives a chapter's length when known;
/// without it only the "starts at 1" part can be checked.
pub fn check_boundaries(verses: &[Verse], verses_count: impl Fn(u16) -> Option<u16>) -> Vec<IntegrityIssue> {
    let mut issues = Vec::new();
    let missing = |issues: &mut Vec<IntegrityIssue>, c: u16, from: u16, to: u16| {
        for n in from..=to {
            issues.push(IntegrityIssue::MissingVerse { verse_key: format!("{c}:{n}") });
        }
    };
    for w in verses.windows(2) {
        let (a, b) = (sort_key(&w[0]), sort_key(&w[1]));
        if a.0 >= b.0 || b.0 == u16::MAX {
            continue;
        }
        if let Some(last) = verses_count(a.0) {
            missing(&mut issues, a.0, a.1 + 1, last);
        }
        for c in a.0 + 1..b.0 {
            if let Some(last) = verses_count(c) {
                missing(&mut issues, c, 1, last);
            } else {
                issues.push(IntegrityIssue::MissingVerse { verse_key: format!("{c}:1") });
            }
        }
        missing(&mut issues, b.0, 1, b.1.saturating_sub(1));
    }
    issues
}

/// Lines per page in the 15-line Madani mushaf.
pub const MUSHAF_LINES: u16 = 15;

/// Whether a verse's words are structurally complete (words present, positions
/// 1..n in order, exactly one trailing `end` marker, as many words as the verse
/// has). Otherwise the verse is rendered from its verse-level text.
pub fn words_structurally_ok(verse: &Verse) -> bool {
    !word_issues(verse, "text_uthmani").iter().any(IntegrityIssue::is_structural_word_issue)
        && !reference_issues(verse).iter().any(IntegrityIssue::is_structural_word_issue)
}

/// `(page, line)` of a word, if both are present and the line is 1..=15.
pub fn word_layout(word: &Word) -> Option<(u16, u16)> {
    match (word.page_number, word.line_number) {
        (Some(p), Some(l)) if p >= 1 && (1..=MUSHAF_LINES).contains(&l) => Some((p, l)),
        _ => None,
    }
}

/// Where a word sits, given where the word before it in the verse sits: its own
/// `(page, line)`. An `end` marker that has none, or whose own lies *before* the
/// word it follows (the API has 84:21's marker one line early), sits with that word.
pub fn slot_after(prev: Option<(u16, u16)>, word: &Word) -> Option<(u16, u16)> {
    match (word_layout(word), prev) {
        (Some(own), Some(prev)) if word.is_end_marker() && own < prev => Some(prev),
        (Some(own), _) => Some(own),
        (None, prev) if word.is_end_marker() => prev,
        (None, _) => None,
    }
}

/// Whether every non-end word of a verse has valid line data, so the verse can
/// be placed on its mushaf lines. (A missing `end` marker position is tolerated:
/// it joins the line of the word before it.)
pub fn verse_layout_ok(verse: &Verse) -> bool {
    verse.words.iter().filter(|w| !w.is_end_marker()).all(|w| word_layout(w).is_some())
}

/// Whether the word's glyph (drawn with the `v2_page` font) belongs to the page it
/// is laid out on. Without `v2_page` the layout page is used for the font.
pub fn glyph_page_ok(word: &Word) -> bool {
    match (word.v2_page, word.page_number) {
        (Some(font_page), Some(page)) => font_page == page,
        _ => true,
    }
}

/// (e) Mushaf layout checks over a sorted list. Verses that already fall back to
/// verse text (structural word issues) are skipped.
pub fn layout_issues(verses: &[Verse]) -> Vec<IntegrityIssue> {
    let mut issues = Vec::new();
    // Per page: number of line runs so far; last (page, line) seen.
    let mut runs: std::collections::HashMap<u16, u16> = std::collections::HashMap::new();
    let mut last: Option<(u16, u16)> = None;
    for v in verses.iter().filter(|v| words_structurally_ok(v)) {
        let key = || v.verse_key.clone();
        let mut placeable = true;
        for w in v.words.iter().filter(|w| !w.is_end_marker()) {
            match (w.page_number, w.line_number) {
                (Some(_), Some(line)) if !(1..=MUSHAF_LINES).contains(&line) => {
                    issues.push(IntegrityIssue::LineOutOfRange { verse_key: key(), position: w.position, line });
                    placeable = false;
                }
                (Some(p), Some(_)) if p >= 1 => {}
                _ => {
                    issues.push(IntegrityIssue::MissingLayout { verse_key: key(), position: w.position });
                    placeable = false;
                }
            }
        }
        for w in v.words.iter().filter(|w| !glyph_page_ok(w)) {
            issues.push(IntegrityIssue::GlyphPageMismatch { verse_key: key(), position: w.position });
        }
        if !placeable {
            continue;
        }
        let mut prev = None;
        for w in &v.words {
            let Some((page, line)) = slot_after(prev, w) else { continue };
            prev = Some((page, line));
            if last == Some((page, line)) {
                continue;
            }
            if let Some((lp, ll)) = last
                && (page < lp || (page == lp && line < ll))
            {
                issues.push(IntegrityIssue::LayoutOutOfOrder { verse_key: key(), page });
            }
            let n = runs.entry(page).or_insert(0);
            *n += 1;
            if *n == MUSHAF_LINES + 1 {
                issues.push(IntegrityIssue::TooManyLines { verse_key: key(), page });
            }
            last = Some((page, line));
        }
    }
    issues
}

/// All issues for a loaded list (already sorted/deduped).
/// `expected` lists `(chapter, span)` that must be fully present; pass it only once
/// everything requested has been loaded.
pub fn check_all(
    verses: &[Verse],
    script_field: &str,
    expected: &[(u16, VerseSpan)],
    verses_count: impl Fn(u16) -> Option<u16>,
) -> Vec<IntegrityIssue> {
    let mut issues = check_gaps(verses);
    let add = |found: Vec<IntegrityIssue>, issues: &mut Vec<IntegrityIssue>| {
        for i in found {
            if !issues.contains(&i) {
                issues.push(i);
            }
        }
    };
    add(check_boundaries(verses, &verses_count), &mut issues);
    for &(chapter, span) in expected {
        add(check_expected(verses, chapter, span), &mut issues);
    }
    for v in verses {
        issues.extend(word_issues(v, script_field));
        issues.extend(reference_issues(v));
    }
    issues.extend(layout_issues(verses));
    issues
}

/// Distinct verse keys of issues matching `pred`, in order.
pub fn keys_where(issues: &[IntegrityIssue], pred: impl Fn(&IntegrityIssue) -> bool) -> Vec<String> {
    let mut keys: Vec<String> = Vec::new();
    for i in issues.iter().filter(|i| pred(i)) {
        if !keys.iter().any(|k| k == i.verse_key()) {
            keys.push(i.verse_key().to_string());
        }
    }
    keys
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const U: &str = "text_uthmani";

    fn word(pos: u16, kind: &str, text: Option<&str>) -> serde_json::Value {
        json!({ "id": pos, "position": pos, "char_type_name": kind, "text_uthmani": text,
                "page_number": 1, "line_number": 1 })
    }

    /// A well-formed verse with `n` words plus an end marker.
    fn verse_with(key: &str, words: Vec<serde_json::Value>) -> Verse {
        let n: u16 = key.split(':').nth(1).unwrap().parse().unwrap();
        serde_json::from_value(json!({
            "id": n, "verse_number": n, "verse_key": key, "text_uthmani": "نص الآية", "words": words
        }))
        .unwrap()
    }

    fn good(key: &str) -> Verse {
        verse_with(
            key,
            vec![word(1, "word", Some("ب")), word(2, "word", Some("س")), word(3, "end", Some("١"))],
        )
    }

    fn keys(vs: &[Verse]) -> Vec<&str> {
        vs.iter().map(|v| v.verse_key.as_str()).collect()
    }

    #[test]
    fn sorts_and_dedups_across_pages() {
        let mut vs = vec![good("2:2"), good("2:1"), good("2:3"), good("2:2"), good("1:7")];
        let issues = sort_dedup(&mut vs);
        assert_eq!(keys(&vs), ["1:7", "2:1", "2:2", "2:3"]);
        assert_eq!(
            issues,
            vec![
                IntegrityIssue::OutOfOrder { verse_key: "2:1".into() },
                IntegrityIssue::Duplicate { verse_key: "2:2".into() },
            ]
        );
        let mut sorted = vec![good("2:1"), good("2:2")];
        assert!(sort_dedup(&mut sorted).is_empty());
    }

    #[test]
    fn page_boundary_gap() {
        // API page 1 ends at 50, page 2 starts at 52: verse 51 is missing.
        let vs = vec![good("2:49"), good("2:50"), good("2:52"), good("2:53")];
        assert_eq!(check_gaps(&vs), vec![IntegrityIssue::MissingVerse { verse_key: "2:51".into() }]);
        // A chapter change is not a gap.
        assert!(check_gaps(&[good("2:286"), good("3:1")]).is_empty());
    }

    #[test]
    fn expected_range_gaps() {
        let vs = vec![good("2:2"), good("2:3")];
        let span = VerseSpan { from: 1, to: 4 };
        assert_eq!(
            check_expected(&vs, 2, span),
            vec![
                IntegrityIssue::MissingVerse { verse_key: "2:1".into() },
                IntegrityIssue::MissingVerse { verse_key: "2:4".into() },
            ]
        );
        assert!(check_expected(&vs, 2, VerseSpan { from: 2, to: 3 }).is_empty());
        // check_all doesn't double-report a gap found both ways.
        let vs = vec![good("2:1"), good("2:3")];
        assert_eq!(
            check_all(&vs, U, &[(2, VerseSpan { from: 1, to: 3 })], |_| None),
            vec![IntegrityIssue::MissingVerse { verse_key: "2:2".into() }]
        );
    }

    #[test]
    fn good_verse_has_no_issues() {
        assert!(word_issues(&good("1:1"), U).is_empty());
        assert!(word_issues(&good("1:1"), "text_indopak").is_empty()); // falls back to uthmani
        assert_eq!(word_count(&good("1:1")), 2);
    }

    #[test]
    fn no_words() {
        let v = verse_with("1:1", vec![]);
        assert_eq!(word_issues(&v, U), vec![IntegrityIssue::NoWords { verse_key: "1:1".into() }]);
        assert!(!words_structurally_ok(&v));
    }

    #[test]
    fn position_gap() {
        let v = verse_with(
            "1:1",
            vec![word(1, "word", Some("a")), word(3, "word", Some("b")), word(4, "end", Some("١"))],
        );
        assert_eq!(
            word_issues(&v, U),
            vec![IntegrityIssue::WordPositionGap { verse_key: "1:1".into(), position: 2 }]
        );
        let v = verse_with("1:1", vec![word(2, "word", Some("a")), word(3, "end", Some("١"))]);
        assert_eq!(
            word_issues(&v, U),
            vec![IntegrityIssue::WordPositionGap { verse_key: "1:1".into(), position: 1 }]
        );
    }

    #[test]
    fn end_marker_rules() {
        let missing = verse_with("1:1", vec![word(1, "word", Some("a"))]);
        assert_eq!(
            word_issues(&missing, U),
            vec![IntegrityIssue::MissingEndMarker { verse_key: "1:1".into() }]
        );
        let not_last = verse_with("1:1", vec![word(1, "end", Some("١")), word(2, "word", Some("a"))]);
        assert_eq!(
            word_issues(&not_last, U),
            vec![IntegrityIssue::EndMarkerNotLast { verse_key: "1:1".into(), position: 1 }]
        );
        let two = verse_with("1:1", vec![word(1, "end", Some("١")), word(2, "end", Some("١"))]);
        assert_eq!(
            word_issues(&two, U),
            vec![IntegrityIssue::ExtraEndMarker { verse_key: "1:1".into(), position: 2 }]
        );
    }

    #[test]
    fn empty_word_text_and_fallbacks() {
        let v =
            verse_with("1:1", vec![word(1, "word", None), word(2, "word", Some("  ")), word(3, "end", None)]);
        assert_eq!(
            word_issues(&v, U),
            vec![
                IntegrityIssue::EmptyWordText { verse_key: "1:1".into(), position: 1 },
                IntegrityIssue::EmptyWordText { verse_key: "1:1".into(), position: 2 },
            ]
        );
        // `text` is the last fallback; the selected script wins when present.
        let w: Word = serde_json::from_value(json!({
            "id": 1, "position": 1, "char_type_name": "word", "text": "t", "text_indopak": "i"
        }))
        .unwrap();
        assert_eq!(word_text(&w, U), Some("t"));
        assert_eq!(word_text(&w, "text_indopak"), Some("i"));
    }

    #[test]
    fn verse_text_fallback() {
        let v: Verse = serde_json::from_value(json!({
            "id": 1, "verse_number": 1, "verse_key": "1:1", "text_indopak": "indo"
        }))
        .unwrap();
        assert_eq!(verse_text(&v, U), Some("indo"));
        assert_eq!(verse_text(&good("1:1"), "text_indopak"), Some("نص الآية"));
    }

    #[test]
    fn keys_filtering() {
        let issues = vec![
            IntegrityIssue::EmptyWordText { verse_key: "2:5".into(), position: 1 },
            IntegrityIssue::EmptyWordText { verse_key: "2:5".into(), position: 2 },
            IntegrityIssue::MissingVerse { verse_key: "2:7".into() },
            IntegrityIssue::NoWords { verse_key: "2:9".into() },
        ];
        assert_eq!(keys_where(&issues, IntegrityIssue::is_word_issue), ["2:5", "2:9"]);
        assert_eq!(keys_where(&issues, |i| matches!(i, IntegrityIssue::MissingVerse { .. })), ["2:7"]);
        assert_eq!(issues[0].to_string(), "2:5: word 1 has no text");
    }

    #[test]
    fn boundary_gap_at_chapter_start_is_flagged() {
        // Juz 30: 78:46 then 79:2 -> 79:1 missing (no chapter lengths needed).
        let vs = vec![good("78:46"), good("79:2")];
        assert_eq!(
            check_boundaries(&vs, |_| None),
            vec![IntegrityIssue::MissingVerse { verse_key: "79:1".into() }]
        );
    }

    #[test]
    fn boundary_gap_at_chapter_end_and_whole_chapter_flagged() {
        let counts = |c: u16| match c {
            112 => Some(4),
            113 => Some(5),
            _ => None,
        };
        // 112:3 then 114:1 -> 112:4 and all of 113 missing.
        let vs = vec![good("112:3"), good("114:1")];
        let keys: Vec<String> = check_boundaries(&vs, counts).iter().map(|i| i.verse_key().to_string()).collect();
        assert_eq!(keys, vec!["112:4", "113:1", "113:2", "113:3", "113:4", "113:5"]);
        // Clean boundary -> nothing.
        assert!(check_boundaries(&[good("112:4"), good("113:1")], counts).is_empty());
    }

    #[test]
    fn check_all_checks_every_expected_span() {
        let vs = vec![good("78:40"), good("79:1")];
        let spans = [(78, VerseSpan { from: 39, to: 40 }), (79, VerseSpan { from: 1, to: 2 })];
        let keys: Vec<String> =
            check_all(&vs, U, &spans, |_| None).iter().map(|i| i.verse_key().to_string()).collect();
        assert_eq!(keys, vec!["78:39", "79:2"]);
    }

    /// A verse whose words (and end marker) sit on `(page, line)` slots.
    fn laid_out(key: &str, slots: &[(u16, u16)]) -> Verse {
        let n = slots.len();
        let words: Vec<serde_json::Value> = slots
            .iter()
            .enumerate()
            .map(|(i, (p, l))| {
                let pos = i as u16 + 1;
                let kind = if i + 1 == n { "end" } else { "word" };
                json!({ "id": pos, "position": pos, "char_type_name": kind, "text_uthmani": "ب",
                        "page_number": p, "line_number": l, "v2_page": p, "code_v2": "\u{fb51}" })
            })
            .collect();
        verse_with(key, words)
    }

    #[test]
    fn layout_ok_and_glyph_page() {
        let v = laid_out("2:1", &[(2, 3), (2, 3), (2, 4)]);
        assert!(verse_layout_ok(&v) && words_structurally_ok(&v));
        assert!(layout_issues(&[v]).is_empty());
        let mut v = laid_out("2:2", &[(2, 4), (2, 4)]);
        v.words[0].v2_page = Some(3);
        assert!(!glyph_page_ok(&v.words[0]));
        assert_eq!(
            layout_issues(&[v]),
            vec![IntegrityIssue::GlyphPageMismatch { verse_key: "2:2".into(), position: 1 }]
        );
    }

    #[test]
    fn missing_or_invalid_line_data() {
        let mut v = laid_out("2:1", &[(2, 3), (2, 3), (2, 4)]);
        v.words[1].line_number = None;
        let mut w = laid_out("2:2", &[(2, 4), (2, 4)]);
        w.words[0].line_number = Some(16);
        assert!(!verse_layout_ok(&v) && !verse_layout_ok(&w));
        let issues = layout_issues(&[v, w]);
        assert_eq!(
            issues,
            vec![
                IntegrityIssue::MissingLayout { verse_key: "2:1".into(), position: 2 },
                IntegrityIssue::LineOutOfRange { verse_key: "2:2".into(), position: 1, line: 16 },
            ]
        );
        assert!(issues.iter().all(IntegrityIssue::is_layout_issue));
        // An end marker without line data is tolerated (it joins the previous line).
        let mut e = laid_out("2:3", &[(2, 5), (2, 5)]);
        e.words[1].line_number = None;
        assert!(verse_layout_ok(&e));
        assert!(layout_issues(&[e]).is_empty());
    }

    #[test]
    fn page_shape_checks() {
        // 16 distinct line runs on one page (line numbers going backwards).
        let mut vs = Vec::new();
        for i in 0..16u16 {
            let line = if i < 15 { i + 1 } else { 1 };
            vs.push(laid_out(&format!("3:{}", i + 1), &[(50, line), (50, line)]));
        }
        let issues = layout_issues(&vs);
        assert!(issues.contains(&IntegrityIssue::TooManyLines { verse_key: "3:16".into(), page: 50 }));
        assert!(issues.contains(&IntegrityIssue::LayoutOutOfOrder { verse_key: "3:16".into(), page: 50 }));
        assert!(issues.iter().all(IntegrityIssue::is_page_shape_issue));
        // Page going backwards.
        let vs = vec![laid_out("3:1", &[(51, 1), (51, 1)]), laid_out("3:2", &[(50, 15), (50, 15)])];
        assert_eq!(
            layout_issues(&vs),
            vec![IntegrityIssue::LayoutOutOfOrder { verse_key: "3:2".into(), page: 50 }]
        );
        // Structurally broken verses are skipped (they fall back to verse text).
        assert!(layout_issues(&[verse_with("3:3", vec![])]).is_empty());
    }

    #[test]
    fn qpc_text_counts_as_word_text() {
        let w: Word = serde_json::from_value(json!({
            "id": 1, "position": 1, "char_type_name": "word", "qpc_uthmani_hafs": "q"
        }))
        .unwrap();
        assert_eq!(word_text(&w, U), Some("q"));
        assert!(!IntegrityIssue::EmptyWordText { verse_key: "1:1".into(), position: 1 }.is_structural_word_issue());
        assert!(IntegrityIssue::NoWords { verse_key: "1:1".into() }.is_structural_word_issue());
    }

    #[test]
    fn word_counts_are_checked_against_the_reference() {
        CHECK_REFERENCE.set(true);
        // 1:1 has four words and its marker.
        let whole = verse_n("1:1", 4);
        assert_eq!(reference_issues(&whole), []);
        assert!(words_structurally_ok(&whole));
        // The same verse without its last word still has contiguous positions…
        let short = verse_n("1:1", 3);
        assert_eq!(word_issues(&short, "text_uthmani"), []);
        // …but is known to be one word short, and is then shown from its verse text.
        let issue = IntegrityIssue::WordCount { verse_key: "1:1".into(), expected: 4, found: 3 };
        assert_eq!(reference_issues(&short), std::slice::from_ref(&issue));
        assert!(issue.is_structural_word_issue() && !words_structurally_ok(&short));
        assert_eq!(issue.to_string(), "1:1: 3 words instead of 4");
        assert_eq!(reference_issues(&verse_n("1:1", 5)).len(), 1, "an extra word is reported too");
        // A verse past the end of its surah does not exist.
        assert_eq!(reference_issues(&verse_n("1:8", 4)), [IntegrityIssue::UnknownVerse { verse_key: "1:8".into() }]);
        assert!(check_all(&[short], "text_uthmani", &[], |_| None).contains(&issue));
        CHECK_REFERENCE.set(false);
    }

    /// A verse with `n` words (positions 1..=n) and its end marker.
    fn verse_n(key: &str, n: u16) -> Verse {
        let number: u16 = key.split(':').nth(1).unwrap().parse().unwrap();
        let mut words: Vec<serde_json::Value> = (1..=n)
            .map(|p| serde_json::json!({ "id": p, "position": p, "char_type_name": "word", "text_uthmani": "x", "page_number": 1, "line_number": 2 }))
            .collect();
        words.push(serde_json::json!({ "id": 99, "position": n + 1, "char_type_name": "end", "text_uthmani": "١", "page_number": 1, "line_number": 2 }));
        serde_json::from_value(serde_json::json!({ "id": 1, "verse_number": number, "verse_key": key, "words": words })).unwrap()
    }

    #[test]
    fn lines_are_counted_against_the_reference() {
        CHECK_REFERENCE.set(true);
        // Page 1 as printed: lines 2..=8, Al-Fatihah's seven verses.
        let per_line: Vec<(u16, usize)> = (1..=15).filter_map(|l| Some((l, reference::words_on_line(1, l)?))).filter(|l| l.1 > 0).collect();
        assert_eq!(per_line.iter().map(|l| l.1).sum::<usize>(), 36);
        // One verse per line count: build verses whose words fill each line exactly.
        let mut verses = Vec::new();
        let mut line_iter = per_line.iter().flat_map(|(l, n)| std::iter::repeat_n(*l, *n));
        for n in 1..=7u16 {
            let words = reference::words_in(1, n).unwrap() + 1;
            let slots: Vec<(u16, u16)> = line_iter.by_ref().take(words).map(|l| (1, l)).collect();
            verses.push(placed(&format!("1:{n}"), &slots));
        }
        assert_eq!(line_issues(&verses, Some(1)), [], "the whole page is there");
        assert_eq!(line_issues(&verses, None), []);
        // Without its last verse the page lacks words on its last lines.
        let short = &verses[..6];
        let issues = line_issues(short, Some(1));
        assert!(!issues.is_empty(), "a page with a verse missing must be reported");
        assert!(issues.iter().all(|i| matches!(i, IntegrityIssue::LineWordCount { page: 1, .. })));
        assert!(issues[0].to_string().starts_with("page 1 line "), "{}", issues[0]);
        // The same six verses as a range: the cut last line is an edge and is not counted.
        assert!(line_issues(short, None).iter().all(|i| !matches!(i, IntegrityIssue::LineWordCount { line: 8, .. })));
        // A word that sits on the wrong line in the middle of a page is noticed in any view.
        let mut moved = verses.clone();
        let before = moved[2].words.last().unwrap().line_number;
        let after = moved[4].words[0].line_number;
        if moved[3].words[0].line_number != before {
            moved[3].words[0].line_number = before;
        } else {
            moved[3].words.last_mut().unwrap().line_number = after;
        }
        assert!(line_issues(&moved, None).iter().any(|i| matches!(i, IntegrityIssue::LineWordCount { .. })));
        CHECK_REFERENCE.set(false);
    }

    /// A verse whose words (the last one its marker) sit on the given `(page, line)` slots.
    fn placed(key: &str, slots: &[(u16, u16)]) -> Verse {
        let number: u16 = key.split(':').nth(1).unwrap().parse().unwrap();
        let last = slots.len() - 1;
        let words: Vec<serde_json::Value> = slots
            .iter()
            .enumerate()
            .map(|(i, (page, line))| serde_json::json!({
                "id": i, "position": i + 1, "char_type_name": if i == last { "end" } else { "word" },
                "text_uthmani": "x", "page_number": page, "line_number": line
            }))
            .collect();
        serde_json::from_value(serde_json::json!({ "id": 1, "verse_number": number, "verse_key": key, "words": words })).unwrap()
    }
}
