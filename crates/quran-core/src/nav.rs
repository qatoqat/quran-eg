//! Navigation shortcuts typed into the search box.
//!
//! Pure (no Dioxus state), so it is unit-tested below. Understands:
//! - verse keys: `2:255`, `2 255`, `2/255`, ranges `2:1-5`, `2/1-5`
//! - chapters: `18`, `surah 18`, `sura 18`, `chapter 18`, `s18`
//! - mushaf pages: `page 3`, `p3`, `p 3`
//! - juz: `juz 30`, `j30`, `para 30`
//! - surah names, case/diacritic-insensitive, ignoring the article
//!   (`al-`, `an-`, `ash-` ...), hyphens and apostrophes: `baqara` -> 2,
//!   `Al Kahf` -> 18, optionally followed by a verse (`kahf 10` -> 18:10).

use crate::api::{Chapter, Error};
use crate::parse::VerseSpan;
use crate::view::View;

pub const CHAPTER_COUNT: u16 = 114;
pub const PAGE_COUNT: u16 = 604;
pub const JUZ_COUNT: u8 = 30;

/// Maximum number of surah-name matches returned.
const MAX_NAME_MATCHES: usize = 3;

/// Where a shortcut leads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavTarget {
    Chapter(u16),
    Verse { chapter: u16, verse: u16 },
    Range { chapter: u16, from: u16, to: u16 },
    Page(u16),
    Juz(u8),
}

impl NavTarget {
    pub fn view(self) -> View {
        let span = |from, to| Some(VerseSpan { from, to });
        match self {
            NavTarget::Chapter(chapter) => View::surah(chapter),
            NavTarget::Verse { chapter, verse } => View::Surah { chapter, span: span(verse, verse) },
            NavTarget::Range { chapter, from, to } => View::Surah { chapter, span: span(from, to) },
            NavTarget::Page(page) => View::Page(page),
            NavTarget::Juz(juz) => View::Juz(juz),
        }
    }

    /// Short badge text, e.g. `2:255`, `Page 3`.
    pub fn badge(self) -> String {
        match self {
            NavTarget::Chapter(c) => format!("Surah {c}"),
            NavTarget::Verse { chapter, verse } => format!("{chapter}:{verse}"),
            NavTarget::Range { chapter, from, to } => format!("{chapter}:{from}-{to}"),
            NavTarget::Page(p) => format!("Page {p}"),
            NavTarget::Juz(j) => format!("Juz {j}"),
        }
    }
}

/// A navigation shortcut with a human-readable label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NavSuggestion {
    pub target: NavTarget,
    /// e.g. "Al-Baqarah (The Cow), verse 255"
    pub label: String,
}

impl NavSuggestion {
    pub fn view(&self) -> View {
        self.target.view()
    }
}

/// The view of a single verse key like `"2:255"` (a search result); `None` if it isn't one.
pub fn verse_view(verse_key: &str) -> Option<View> {
    let (c, v) = verse_key.split_once(':')?;
    let (chapter, verse) = (c.parse::<u16>().ok()?, v.parse::<u16>().ok()?);
    View::Surah { chapter, span: VerseSpan::single(verse).ok() }.checked().ok()
}

/// User-facing message for a failed search request.
pub fn describe_error(e: &Error) -> String {
    match e {
        Error::Status { status, .. } if matches!(status, 401 | 403 | 404 | 410 | 501) => format!(
            "Search is unavailable right now: the Quran.com search endpoint returned HTTP {status}. \
             It may have been retired or now require authentication."
        ),
        Error::Status { status, .. } => {
            format!("Search failed: the server returned HTTP {status}. Please try again later.")
        }
        Error::Decode(_) => "Search is unavailable right now: the search endpoint returned a response \
             this app does not understand."
            .to_string(),
        Error::Http(_) => {
            "Search failed: could not reach the server. Check your connection and try again.".to_string()
        }
    }
}

/// "1 result" / "5 results".
pub fn plural(n: u32, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// Parse `query` into navigation shortcuts, best first. `chapters` may be empty
/// (not loaded yet / failed): numeric shortcuts still work, names don't.
pub fn parse(query: &str, chapters: &[Chapter]) -> Vec<NavSuggestion> {
    let q = query.trim();
    if q.is_empty() {
        return Vec::new();
    }
    let mut targets: Vec<NavTarget> = Vec::new();

    if let Some(t) = parse_verse_key(q) {
        targets.push(t);
    } else if let Some(n) = parse_number(q) {
        // Bare number: surah first, then page and juz with the same number.
        if (1..=CHAPTER_COUNT).contains(&n) {
            targets.push(NavTarget::Chapter(n));
        }
        if (1..=PAGE_COUNT).contains(&n) {
            targets.push(NavTarget::Page(n));
        }
        if (1..=JUZ_COUNT as u16).contains(&n) {
            targets.push(NavTarget::Juz(n as u8));
        }
    } else if let Some(t) = parse_keyword(q) {
        targets.push(t);
    } else {
        targets.extend(match_names(q, chapters));
    }

    targets
        .into_iter()
        .filter(|t| valid(*t, chapters))
        .map(|target| NavSuggestion { target, label: label(target, chapters) })
        .collect()
}

fn chapter_info(id: u16, chapters: &[Chapter]) -> Option<&Chapter> {
    chapters.iter().find(|c| c.id == id)
}

fn valid(t: NavTarget, chapters: &[Chapter]) -> bool {
    let verse_ok = |chapter: u16, verse: u16| {
        verse >= 1 && chapter_info(chapter, chapters).is_none_or(|c| verse <= c.verses_count)
    };
    match t {
        NavTarget::Chapter(c) => (1..=CHAPTER_COUNT).contains(&c),
        NavTarget::Verse { chapter, verse } => {
            (1..=CHAPTER_COUNT).contains(&chapter) && verse_ok(chapter, verse)
        }
        NavTarget::Range { chapter, from, to } => {
            (1..=CHAPTER_COUNT).contains(&chapter)
                && from < to
                && verse_ok(chapter, from)
                && verse_ok(chapter, to)
        }
        NavTarget::Page(p) => (1..=PAGE_COUNT).contains(&p),
        NavTarget::Juz(j) => (1..=JUZ_COUNT).contains(&j),
    }
}

fn label(t: NavTarget, chapters: &[Chapter]) -> String {
    let name = |id: u16| match chapter_info(id, chapters) {
        Some(c) => format!("Surah {} ({})", c.name_simple, c.translated_name.name),
        None => format!("Surah {id}"),
    };
    match t {
        NavTarget::Chapter(c) => name(c),
        NavTarget::Verse { chapter, verse } => format!("{}, verse {verse}", name(chapter)),
        NavTarget::Range { chapter, from, to } => {
            format!("{}, verses {from}-{to}", name(chapter))
        }
        NavTarget::Page(p) => format!("Mushaf page {p}"),
        NavTarget::Juz(j) => format!("Juz {j}"),
    }
}

fn parse_number(s: &str) -> Option<u16> {
    let s = s.trim();
    if s.is_empty() || s.len() > 4 || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

/// `verse` or `from-to`.
fn parse_verse_part(chapter: u16, s: &str) -> Option<NavTarget> {
    let s = s.trim();
    if let Some((a, b)) = s.split_once('-') {
        let (from, to) = (parse_number(a)?, parse_number(b)?);
        if from == to {
            return Some(NavTarget::Verse { chapter, verse: from });
        }
        Some(NavTarget::Range { chapter, from, to })
    } else {
        Some(NavTarget::Verse { chapter, verse: parse_number(s)? })
    }
}

/// `2:255`, `2 255`, `2/255`, `2:1-5`, `2 : 255`.
fn parse_verse_key(q: &str) -> Option<NavTarget> {
    let (a, b) = q
        .split_once(':')
        .or_else(|| q.split_once('/'))
        .or_else(|| q.split_once(char::is_whitespace))?;
    let chapter = parse_number(a)?;
    parse_verse_part(chapter, b)
}

/// `surah 18`, `s18`, `page 3`, `p3`, `juz 30`, `j30`, `para 30`.
fn parse_keyword(q: &str) -> Option<NavTarget> {
    let lower = q.to_lowercase();
    let split = lower.find(|c: char| c.is_ascii_digit())?;
    let (word, rest) = lower.split_at(split);
    let word = word.trim().trim_end_matches(['.', ':', '#']).trim();
    match word {
        "surah" | "sura" | "surat" | "chapter" | "ch" | "s" => {
            let rest = rest.trim();
            // `surah 2:255` / `surah 2 255`
            if let Some(t) = parse_verse_key(rest) {
                return Some(t);
            }
            Some(NavTarget::Chapter(parse_number(rest)?))
        }
        "page" | "pg" | "p" => Some(NavTarget::Page(parse_number(rest)?)),
        "juz" | "juzz" | "para" | "j" => {
            let n = parse_number(rest)?;
            Some(NavTarget::Juz(u8::try_from(n).ok()?))
        }
        _ => None,
    }
}

/// Fold a Latin-transliteration character to plain ASCII lowercase.
/// Returns `None` for characters that should be dropped (apostrophes, ʿayn, hamza, ...).
fn fold_char(c: char) -> Option<char> {
    let folded = match c {
        'ā' | 'á' | 'à' | 'â' | 'ä' | 'ã' | 'å' | 'Ā' | 'Á' | 'À' | 'Â' | 'Ä' | 'Ã' | 'Å' => {
            'a'
        }
        'ī' | 'í' | 'ì' | 'î' | 'ï' | 'Ī' | 'Í' | 'Ì' | 'Î' | 'Ï' => 'i',
        'ū' | 'ú' | 'ù' | 'û' | 'ü' | 'Ū' | 'Ú' | 'Ù' | 'Û' | 'Ü' => 'u',
        'ē' | 'é' | 'è' | 'ê' | 'ë' | 'Ē' | 'É' | 'È' | 'Ê' | 'Ë' => 'e',
        'ō' | 'ó' | 'ò' | 'ô' | 'ö' | 'Ō' | 'Ó' | 'Ò' | 'Ô' | 'Ö' => 'o',
        'ḥ' | 'Ḥ' | 'ḫ' | 'Ḫ' | 'ĥ' | 'Ĥ' => 'h',
        'ṣ' | 'Ṣ' | 'š' | 'Š' | 'ś' | 'Ś' | 'ş' | 'Ş' => 's',
        'ḍ' | 'Ḍ' | 'ḏ' | 'Ḏ' => 'd',
        'ṭ' | 'Ṭ' | 'ṯ' | 'Ṯ' | 'ţ' | 'Ţ' => 't',
        'ẓ' | 'Ẓ' | 'ž' | 'Ž' | 'ż' | 'Ż' | 'ẕ' | 'Ẕ' => 'z',
        'ġ' | 'Ġ' | 'ǧ' | 'Ǧ' | 'ğ' | 'Ğ' => 'g',
        'ç' | 'Ç' => 'c',
        'ñ' | 'Ñ' => 'n',
        // apostrophes, ʿayn, hamza, backticks, combining marks
        '\'' | '`' | '’' | '‘' | 'ʿ' | 'ʾ' | 'ʼ' | 'ʻ' | '´' => return None,
        c if ('\u{0300}'..='\u{036f}').contains(&c) => return None,
        c => return c.to_lowercase().next(),
    };
    Some(folded)
}

/// Leading words ignored in names: Arabic definite article spellings (with
/// sun-letter assimilation), "Ali" as in "Ali 'Imran", and English "the".
const ARTICLES: &[&str] =
    &["al", "el", "an", "ar", "as", "at", "ad", "ash", "az", "adh", "ath", "ali", "aal", "the"];

/// Normalize a surah name or query: fold diacritics, split on non-alphanumerics,
/// drop a leading "surah" word and a leading article, then join without separators.
pub fn normalize_name(s: &str) -> String {
    let folded: String = s.chars().filter_map(fold_char).collect();
    let mut tokens: Vec<&str> =
        folded.split(|c: char| !c.is_alphanumeric()).filter(|t| !t.is_empty()).collect();
    if tokens.len() > 1 && matches!(tokens[0], "surah" | "sura" | "surat" | "chapter") {
        tokens.remove(0);
    }
    if tokens.len() > 1 && ARTICLES.contains(&tokens[0]) {
        tokens.remove(0);
    }
    tokens.concat()
}

/// Match quality, higher is better.
fn name_score(query: &str, name: &str) -> u8 {
    let name = normalize_name(name);
    if name.is_empty() {
        0
    } else if name == query {
        3
    } else if query.len() >= 3 && name.starts_with(query) {
        2
    } else if query.len() >= 4 && name.contains(query) {
        1
    } else {
        0
    }
}

fn best_chapter_score(query: &str, c: &Chapter) -> u8 {
    [
        c.name_simple.as_str(),
        c.name_complex.as_str(),
        c.translated_name.name.as_str(),
        c.name_arabic.as_str(),
    ]
    .iter()
    .map(|n| name_score(query, n))
    .max()
    .unwrap_or(0)
}

fn match_names(q: &str, chapters: &[Chapter]) -> Vec<NavTarget> {
    if chapters.is_empty() {
        return Vec::new();
    }
    // Optional trailing verse/range: `kahf 10`, `baqara 1-5`.
    let (name_part, verse_part) = match q.trim().rsplit_once(char::is_whitespace) {
        Some((name, tail)) if tail.starts_with(|c: char| c.is_ascii_digit()) => (name, Some(tail)),
        _ => (q, None),
    };
    let query = normalize_name(name_part);
    if query.is_empty() {
        return Vec::new();
    }
    let mut scored: Vec<(u8, u16)> = chapters
        .iter()
        .map(|c| (best_chapter_score(&query, c), c.id))
        .filter(|(s, _)| *s > 0)
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    scored
        .into_iter()
        .take(MAX_NAME_MATCHES)
        .filter_map(|(_, id)| match verse_part {
            Some(v) => parse_verse_part(id, v),
            None => Some(NavTarget::Chapter(id)),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::TranslatedName;

    fn chapter(id: u16, simple: &str, complex: &str, translated: &str, verses: u16) -> Chapter {
        Chapter {
            id,
            revelation_place: "madinah".into(),
            revelation_order: 1,
            bismillah_pre: true,
            name_simple: simple.into(),
            name_complex: complex.into(),
            name_arabic: String::new(),
            verses_count: verses,
            pages: vec![],
            translated_name: TranslatedName { language_name: None, name: translated.into() },
        }
    }

    fn sample_chapters() -> Vec<Chapter> {
        vec![
            chapter(1, "Al-Fatihah", "Al-Fātiĥah", "The Opener", 7),
            chapter(2, "Al-Baqarah", "Al-Baqarah", "The Cow", 286),
            chapter(3, "Ali 'Imran", "Āli `Imrān", "Family of Imran", 200),
            chapter(18, "Al-Kahf", "Al-Kahf", "The Cave", 110),
            chapter(20, "Taha", "Ţāhā", "Ta-Ha", 135),
            chapter(36, "Ya-Sin", "Yā-Sīn", "Ya Sin", 83),
            chapter(55, "Ar-Rahman", "Ar-Raĥmān", "The Beneficent", 78),
            chapter(112, "Al-Ikhlas", "Al-'Ikhlāş", "The Sincerity", 4),
            chapter(114, "An-Nas", "An-Nās", "Mankind", 6),
        ]
    }

    fn targets(q: &str) -> Vec<NavTarget> {
        parse(q, &sample_chapters()).into_iter().map(|s| s.target).collect()
    }

    fn first(q: &str) -> Option<NavTarget> {
        targets(q).into_iter().next()
    }

    #[test]
    fn verse_keys() {
        let v = Some(NavTarget::Verse { chapter: 2, verse: 255 });
        assert_eq!(first("2:255"), v);
        assert_eq!(first("2 255"), v);
        assert_eq!(first("2/255"), v);
        assert_eq!(first(" 2 : 255 "), v);
        assert_eq!(first("surah 2:255"), v);
    }

    #[test]
    fn ranges() {
        assert_eq!(first("2:1-5"), Some(NavTarget::Range { chapter: 2, from: 1, to: 5 }));
        assert_eq!(first("2/1-5"), Some(NavTarget::Range { chapter: 2, from: 1, to: 5 }));
        assert_eq!(first("2:5-5"), Some(NavTarget::Verse { chapter: 2, verse: 5 }));
        assert_eq!(first("2:5-1"), None, "reversed ranges are rejected");
    }

    #[test]
    fn rejects_out_of_range_verses() {
        assert_eq!(first("1:8"), None, "Al-Fatihah has 7 verses");
        assert_eq!(first("115:1"), None);
        assert_eq!(first("2:0"), None);
        assert_eq!(first("2:280-290"), None);
        // Without chapter data only the chapter number is validated.
        assert_eq!(parse("1:8", &[]).len(), 1);
    }

    #[test]
    fn bare_numbers() {
        assert_eq!(
            targets("18"),
            vec![NavTarget::Chapter(18), NavTarget::Page(18), NavTarget::Juz(18)]
        );
        assert_eq!(targets("300"), vec![NavTarget::Page(300)]);
        assert_eq!(targets("605"), vec![]);
        assert_eq!(targets("0"), vec![]);
    }

    #[test]
    fn keywords() {
        assert_eq!(first("surah 18"), Some(NavTarget::Chapter(18)));
        assert_eq!(first("Sura 18"), Some(NavTarget::Chapter(18)));
        assert_eq!(first("s18"), Some(NavTarget::Chapter(18)));
        assert_eq!(first("page 3"), Some(NavTarget::Page(3)));
        assert_eq!(first("p3"), Some(NavTarget::Page(3)));
        assert_eq!(first("P 3"), Some(NavTarget::Page(3)));
        assert_eq!(first("page 605"), None);
        assert_eq!(first("juz 30"), Some(NavTarget::Juz(30)));
        assert_eq!(first("j30"), Some(NavTarget::Juz(30)));
        assert_eq!(first("para 1"), Some(NavTarget::Juz(1)));
        assert_eq!(first("juz 31"), None);
        assert_eq!(first("juz 300"), None);
    }

    #[test]
    fn surah_names() {
        assert_eq!(first("baqara"), Some(NavTarget::Chapter(2)));
        assert_eq!(first("Al Kahf"), Some(NavTarget::Chapter(18)));
        assert_eq!(first("al-kahf"), Some(NavTarget::Chapter(18)));
        assert_eq!(first("KAHF"), Some(NavTarget::Chapter(18)));
        assert_eq!(first("rahman"), Some(NavTarget::Chapter(55)));
        assert_eq!(first("Ar-Raĥmān"), Some(NavTarget::Chapter(55)));
        assert_eq!(first("imran"), Some(NavTarget::Chapter(3)));
        assert_eq!(first("al imran"), Some(NavTarget::Chapter(3)));
        assert_eq!(first("yasin"), Some(NavTarget::Chapter(36)));
        assert_eq!(first("ya-sin"), Some(NavTarget::Chapter(36)));
        assert_eq!(first("taha"), Some(NavTarget::Chapter(20)));
        assert_eq!(first("nas"), Some(NavTarget::Chapter(114)));
        assert_eq!(first("ikhlas"), Some(NavTarget::Chapter(112)));
        assert_eq!(first("surah kahf"), Some(NavTarget::Chapter(18)));
        assert_eq!(first("the cow"), Some(NavTarget::Chapter(2)));
        assert_eq!(first("cave"), Some(NavTarget::Chapter(18)));
    }

    #[test]
    fn surah_name_with_verse() {
        assert_eq!(first("kahf 10"), Some(NavTarget::Verse { chapter: 18, verse: 10 }));
        assert_eq!(first("al-baqarah 1-5"), Some(NavTarget::Range { chapter: 2, from: 1, to: 5 }));
        assert_eq!(first("kahf 111"), None);
    }

    #[test]
    fn non_matches() {
        assert!(targets("").is_empty());
        assert!(targets("   ").is_empty());
        assert!(targets("al").is_empty());
        assert!(targets("mercy and light").is_empty());
        assert!(targets("ba").is_empty(), "too short for prefix matching");
    }

    #[test]
    fn exact_beats_prefix() {
        let chapters = vec![
            chapter(110, "An-Nasr", "An-Naşr", "The Divine Support", 3),
            chapter(114, "An-Nas", "An-Nās", "Mankind", 6),
        ];
        let t: Vec<_> = parse("nas", &chapters).into_iter().map(|s| s.target).collect();
        assert_eq!(t, vec![NavTarget::Chapter(114), NavTarget::Chapter(110)]);
    }

    #[test]
    fn arabic_names() {
        let mut chapters = sample_chapters();
        chapters[3].name_arabic = "الكهف".into();
        let t: Vec<_> = parse("الكهف", &chapters).into_iter().map(|s| s.target).collect();
        assert_eq!(t, vec![NavTarget::Chapter(18)]);
    }

    #[test]
    fn routes_and_labels() {
        let s = parse("2:255", &sample_chapters());
        assert_eq!(s[0].view().to_string(), "/2/255");
        assert_eq!(NavTarget::Range { chapter: 2, from: 1, to: 5 }.view().to_string(), "/2/1-5");
        assert_eq!(NavTarget::Chapter(18).view(), View::surah(18));
        assert_eq!(NavTarget::Page(3).view(), View::Page(3));
        assert_eq!(NavTarget::Juz(30).view(), View::Juz(30));
        assert_eq!(verse_view("2:255").map(|v| v.to_string()).as_deref(), Some("/2/255"));
        assert_eq!((verse_view("2:x"), verse_view("0:1"), verse_view("2:999")), (None, None, None));
        let e = Error::Status { status: 403, body: String::new() };
        assert!(describe_error(&e).contains("require authentication"));
        let e = Error::Status { status: 500, body: String::new() };
        assert!(describe_error(&e).contains("HTTP 500"));
        let e = Error::Decode(serde_json::from_str::<u8>("x").unwrap_err());
        assert!(describe_error(&e).contains("does not understand"));
        assert_eq!((plural(1, "result", "results"), plural(0, "result", "results")), ("1 result".into(), "0 results".into()));
        assert_eq!(parse("18", &[])[0].label, "Surah 18");
    }

    #[test]
    fn normalizes_names() {
        assert_eq!(normalize_name("Al-Fātiĥah"), "fatihah");
        assert_eq!(normalize_name("Āli `Imrān"), "imran");
        assert_eq!(normalize_name("Al-'Ikhlāş"), "ikhlas");
        assert_eq!(normalize_name("Al"), "al");
    }
}
