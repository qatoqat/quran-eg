//! Pure parsing of verse references and pagination math (unit-tested).
//!
//! Accepted `chapter` segment forms: `"2"`, `"2:255"`, `"2:1-5"`, `"2:1:5"`, `"2-1-5"`.
//! Accepted `range` segment forms: `"255"`, `"1-5"`.
//! Canonical URLs are `/2`, `/2/255` and `/2/1-5`.

use std::fmt;
use std::ops::RangeInclusive;

/// Number of surahs in the Quran.
pub const CHAPTER_COUNT: u16 = 114;
/// Number of pages in the Madani mushaf.
pub const MUSHAF_PAGES: u16 = 604;
/// Number of ajzāʾ.
pub const JUZ_COUNT: u8 = 30;
/// Verses requested per API page (the API maximum).
pub const PER_PAGE: u32 = 50;

/// An inclusive span of verse numbers within one chapter (`from <= to`, both `>= 1`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerseSpan {
    pub from: u16,
    pub to: u16,
}

impl VerseSpan {
    pub fn new(from: u16, to: u16) -> Result<Self, ParseError> {
        if from == 0 || to == 0 {
            return Err(ParseError::InvalidVerse);
        }
        if from > to {
            return Err(ParseError::ReversedRange);
        }
        Ok(Self { from, to })
    }

    pub fn single(verse: u16) -> Result<Self, ParseError> {
        Self::new(verse, verse)
    }

    pub fn is_single(&self) -> bool {
        self.from == self.to
    }

    pub fn contains(&self, verse: u16) -> bool {
        (self.from..=self.to).contains(&verse)
    }

    /// Canonical `range` URL segment: `"255"` or `"1-5"`.
    pub fn segment(&self) -> String {
        if self.is_single() { self.from.to_string() } else { format!("{}-{}", self.from, self.to) }
    }

    /// Check against the chapter's verse count: reject a span starting past the
    /// end, clamp one that ends past it.
    pub fn clamp_to(self, verses_count: u16) -> Result<Self, ParseError> {
        if self.from > verses_count {
            return Err(ParseError::VerseOutOfRange { verses_count });
        }
        Ok(Self { from: self.from, to: self.to.min(verses_count) })
    }
}

/// A reference to a whole chapter or a span of verses in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerseRef {
    pub chapter: u16,
    pub span: Option<VerseSpan>,
}

impl VerseRef {
    /// Canonical path (without leading `/`): `"2"`, `"2/255"`, `"2/1-5"`.
    pub fn canonical_path(&self) -> String {
        match self.span {
            None => self.chapter.to_string(),
            Some(s) => format!("{}/{}", self.chapter, s.segment()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseError {
    Malformed,
    ChapterOutOfRange,
    InvalidVerse,
    ReversedRange,
    VerseOutOfRange { verses_count: u16 },
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::Malformed => write!(f, "That doesn't look like a surah or verse reference."),
            ParseError::ChapterOutOfRange => {
                write!(f, "Surah numbers go from 1 to {CHAPTER_COUNT}.")
            }
            ParseError::InvalidVerse => write!(f, "Verse numbers start at 1."),
            ParseError::ReversedRange => {
                write!(f, "The first verse of a range must not come after the last one.")
            }
            ParseError::VerseOutOfRange { verses_count } => {
                write!(f, "This surah has only {verses_count} verses.")
            }
        }
    }
}

fn parse_num(s: &str) -> Result<u16, ParseError> {
    let s = s.trim();
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return Err(ParseError::Malformed);
    }
    // Overlong numbers are certainly out of range; saturate so callers reject them.
    Ok(s.parse::<u16>().unwrap_or(u16::MAX))
}

/// Parse and validate a chapter number (1..=114).
pub fn parse_chapter_number(s: &str) -> Result<u16, ParseError> {
    let n = parse_num(s)?;
    if (1..=CHAPTER_COUNT).contains(&n) { Ok(n) } else { Err(ParseError::ChapterOutOfRange) }
}

/// Parse the `range` URL segment: `"255"` or `"1-5"`.
pub fn parse_range(s: &str) -> Result<VerseSpan, ParseError> {
    match s.split_once('-') {
        None => VerseSpan::single(parse_num(s)?),
        Some((a, b)) => VerseSpan::new(parse_num(a)?, parse_num(b)?),
    }
}

/// Parse the `chapter` URL segment, including legacy key forms.
pub fn parse_chapter_segment(s: &str) -> Result<VerseRef, ParseError> {
    let s = s.trim();
    let parts: Vec<&str> = s.split([':', '-']).collect();
    let colon = s.contains(':');
    let (chapter, span) = match parts.as_slice() {
        [c] => (*c, None),
        // "2:255" (but not "2-255", which is ambiguous and never was a valid URL)
        [c, v] if colon => (*c, Some(VerseSpan::single(parse_num(v)?)?)),
        // "2:1-5", "2:1:5", "2-1-5"
        [c, a, b] => (*c, Some(VerseSpan::new(parse_num(a)?, parse_num(b)?)?)),
        _ => return Err(ParseError::Malformed),
    };
    Ok(VerseRef { chapter: parse_chapter_number(chapter)?, span })
}

/// Parse `chapter` + `range` URL segments (`/2/1-5`).
pub fn parse_chapter_and_range(chapter: &str, range: &str) -> Result<VerseRef, ParseError> {
    Ok(VerseRef { chapter: parse_chapter_number(chapter)?, span: Some(parse_range(range)?) })
}

/// Validate a mushaf page number (1..=604).
pub fn valid_mushaf_page(page: u16) -> bool {
    (1..=MUSHAF_PAGES).contains(&page)
}

/// Validate a juz number (1..=30).
pub fn valid_juz(juz: u8) -> bool {
    (1..=JUZ_COUNT).contains(&juz)
}

// ---------------------------------------------------------------------------
// Pagination math for `/verses/by_chapter` (verses are numbered from 1 and the
// API returns `per_page` of them per page, so verse `n` is on page
// `(n - 1) / per_page + 1`).

/// 1-based API page that contains verse `verse` of a chapter.
pub fn api_page_of(verse: u16, per_page: u32) -> u32 {
    let per_page = per_page.max(1);
    (u32::from(verse.max(1)) - 1) / per_page + 1
}

/// API pages that need to be fetched to show `span`.
pub fn api_pages_for(span: VerseSpan, per_page: u32) -> RangeInclusive<u32> {
    api_page_of(span.from, per_page)..=api_page_of(span.to, per_page)
}

/// The next page to fetch given the API's `next_page`, capped at `last` (if any).
pub fn next_page_within(api_next: Option<u32>, last: Option<u32>) -> Option<u32> {
    match (api_next, last) {
        (Some(n), Some(l)) if n > l => None,
        (n, _) => n,
    }
}

/// Human label for a chapter's `revelation_place` (`"makkah"` / `"madinah"`).
pub fn revelation_label(place: &str) -> &'static str {
    match place.to_ascii_lowercase().as_str() {
        "makkah" | "mecca" | "meccan" => "Meccan",
        "madinah" | "medina" | "medinan" => "Medinan",
        _ => "",
    }
}

/// Parse a juz `verse_mapping` (`{"2": "142-252"}`) into `(chapter, span)`
/// pairs sorted by chapter number (the map's keys sort as strings).
pub fn juz_mapping<'a>(mapping: impl IntoIterator<Item = (&'a String, &'a String)>) -> Vec<(u16, VerseSpan)> {
    let mut out: Vec<(u16, VerseSpan)> = mapping
        .into_iter()
        .filter_map(|(c, r)| Some((parse_chapter_number(c).ok()?, parse_range(r).ok()?)))
        .collect();
    out.sort_by_key(|(c, _)| *c);
    out
}

// ---------------------------------------------------------------------------
// quran.com-style surah slugs: `/an-najm`, `/al-baqarah/255`.

/// Fold a Latin letter with diacritics (as used in transliterated surah names,
/// e.g. `ā`, `ĥ`, `ṣ`) to plain ASCII. Other characters are returned unchanged.
fn fold_diacritic(c: char) -> char {
    match c {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' | 'ạ' => 'a',
        'À' | 'Á' | 'Â' | 'Ã' | 'Ä' | 'Å' | 'Ā' | 'Ă' | 'Ą' | 'Ạ' => 'a',
        'ç' | 'ć' | 'č' | 'Ç' | 'Ć' | 'Č' => 'c',
        'ď' | 'ḍ' | 'ḏ' | 'Ď' | 'Ḍ' | 'Ḏ' => 'd',
        'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' | 'È' | 'É' | 'Ê' | 'Ë' | 'Ē' => 'e',
        'ğ' | 'ġ' | 'Ğ' | 'Ġ' => 'g',
        'ĥ' | 'ḥ' | 'ḫ' | 'ħ' | 'Ĥ' | 'Ḥ' | 'Ḫ' | 'Ħ' => 'h',
        'ì' | 'í' | 'î' | 'ï' | 'ī' | 'ĭ' | 'į' | 'ı' | 'Ì' | 'Í' | 'Î' | 'Ï' | 'Ī' | 'İ' => 'i',
        'ñ' | 'ń' | 'ň' | 'Ñ' | 'Ń' | 'Ň' => 'n',
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ō' | 'ŏ' | 'ő' | 'Ò' | 'Ó' | 'Ô' | 'Õ' | 'Ö' | 'Ō' => 'o',
        'ŕ' | 'ř' | 'Ŕ' | 'Ř' => 'r',
        'ś' | 'š' | 'ş' | 'ṣ' | 'Ś' | 'Š' | 'Ş' | 'Ṣ' => 's',
        'ť' | 'ţ' | 'ṭ' | 'Ť' | 'Ţ' | 'Ṭ' => 't',
        'ù' | 'ú' | 'û' | 'ü' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'Ù' | 'Ú' | 'Û' | 'Ü' | 'Ū' => 'u',
        'ý' | 'ÿ' | 'Ý' => 'y',
        'ź' | 'ż' | 'ž' | 'ẓ' | 'ẕ' | 'Ź' | 'Ż' | 'Ž' | 'Ẓ' => 'z',
        _ => c,
    }
}

/// Apostrophe-like marks dropped from slugs (`Al-Ma'idah` → `al-maidah`, like quran.com).
fn is_apostrophe(c: char) -> bool {
    matches!(c, '\'' | '’' | '‘' | '`' | 'ʼ' | 'ʾ' | 'ʿ' | 'ʻ' | '´')
}

/// URL slug of a transliterated surah name: lowercase ASCII, diacritics folded,
/// apostrophes dropped, every other run of non-alphanumerics → one `-`.
/// `"An-Najm"` → `"an-najm"`, `"Ali 'Imran"` → `"ali-imran"`, `"Al-Fātiĥah"` → `"al-fatihah"`.
pub fn slugify(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut pending_hyphen = false;
    for c in name.chars().map(fold_diacritic) {
        if is_apostrophe(c) {
            continue;
        }
        if c.is_ascii_alphanumeric() {
            if pending_hyphen && !out.is_empty() {
                out.push('-');
            }
            pending_hyphen = false;
            out.push(c.to_ascii_lowercase());
        } else {
            pending_hyphen = true;
        }
    }
    out
}

/// Slug reduced to letters and digits only, so `al-baqarah`, `albaqarah` and
/// `al-ma-idah` (apostrophe written as a hyphen) all compare equal.
fn slug_key(s: &str) -> String {
    slugify(s).chars().filter(char::is_ascii_alphanumeric).collect()
}

/// Whether a URL segment could be a surah slug (has a letter; not a key like `2:255`).
pub fn looks_like_slug(segment: &str) -> bool {
    let s = segment.trim();
    !s.is_empty() && s.len() <= 64 && s.chars().any(char::is_alphabetic) && !s.contains(':')
}

/// Find the chapter whose `name_simple` or `name_complex` matches `slug`
/// (case-, diacritic-, apostrophe- and hyphen-insensitive). `names` yields
/// `(chapter id, name_simple, name_complex)`.
pub fn match_slug<'a>(slug: &str, names: impl IntoIterator<Item = (u16, &'a str, &'a str)>) -> Option<u16> {
    let wanted = slug_key(slug);
    if wanted.is_empty() {
        return None;
    }
    names
        .into_iter()
        .find(|(_, simple, complex)| slug_key(simple) == wanted || slug_key(complex) == wanted)
        .map(|(id, _, _)| id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(from: u16, to: u16) -> Option<VerseSpan> {
        Some(VerseSpan { from, to })
    }

    #[test]
    fn chapter_segment_forms() {
        assert_eq!(parse_chapter_segment("2"), Ok(VerseRef { chapter: 2, span: None }));
        assert_eq!(parse_chapter_segment("2:255"), Ok(VerseRef { chapter: 2, span: span(255, 255) }));
        assert_eq!(parse_chapter_segment("2:1-5"), Ok(VerseRef { chapter: 2, span: span(1, 5) }));
        assert_eq!(parse_chapter_segment("2:1:5"), Ok(VerseRef { chapter: 2, span: span(1, 5) }));
        assert_eq!(parse_chapter_segment("2-1-5"), Ok(VerseRef { chapter: 2, span: span(1, 5) }));
        assert_eq!(parse_chapter_segment("114"), Ok(VerseRef { chapter: 114, span: None }));
        assert_eq!(parse_chapter_segment("002"), Ok(VerseRef { chapter: 2, span: None }));
    }

    #[test]
    fn chapter_segment_errors() {
        assert_eq!(parse_chapter_segment("0"), Err(ParseError::ChapterOutOfRange));
        assert_eq!(parse_chapter_segment("115"), Err(ParseError::ChapterOutOfRange));
        assert_eq!(parse_chapter_segment("99999999"), Err(ParseError::ChapterOutOfRange));
        assert_eq!(parse_chapter_segment(""), Err(ParseError::Malformed));
        assert_eq!(parse_chapter_segment("abc"), Err(ParseError::Malformed));
        assert_eq!(parse_chapter_segment("-2"), Err(ParseError::Malformed));
        assert_eq!(parse_chapter_segment("2-255"), Err(ParseError::Malformed));
        assert_eq!(parse_chapter_segment("2:"), Err(ParseError::Malformed));
        assert_eq!(parse_chapter_segment("2:1-2-3"), Err(ParseError::Malformed));
        assert_eq!(parse_chapter_segment("2:0"), Err(ParseError::InvalidVerse));
        assert_eq!(parse_chapter_segment("2:5-1"), Err(ParseError::ReversedRange));
        assert_eq!(parse_chapter_segment("2:+5"), Err(ParseError::Malformed));
    }

    #[test]
    fn range_segment() {
        assert_eq!(parse_range("255"), Ok(VerseSpan { from: 255, to: 255 }));
        assert_eq!(parse_range("1-5"), Ok(VerseSpan { from: 1, to: 5 }));
        assert_eq!(parse_range("5-5"), Ok(VerseSpan { from: 5, to: 5 }));
        assert_eq!(parse_range("0"), Err(ParseError::InvalidVerse));
        assert_eq!(parse_range("0-3"), Err(ParseError::InvalidVerse));
        assert_eq!(parse_range("7-3"), Err(ParseError::ReversedRange));
        assert_eq!(parse_range("1-"), Err(ParseError::Malformed));
        assert_eq!(parse_range("x"), Err(ParseError::Malformed));
        assert_eq!(parse_range("1-2-3"), Err(ParseError::Malformed));
    }

    #[test]
    fn chapter_and_range() {
        assert_eq!(parse_chapter_and_range("2", "1-5"), Ok(VerseRef { chapter: 2, span: span(1, 5) }));
        assert_eq!(parse_chapter_and_range("200", "1"), Err(ParseError::ChapterOutOfRange));
        assert_eq!(parse_chapter_and_range("2:3", "1"), Err(ParseError::Malformed));
    }

    #[test]
    fn canonical_paths() {
        let r = |s| parse_chapter_segment(s).unwrap().canonical_path();
        assert_eq!(r("2"), "2");
        assert_eq!(r("2:255"), "2/255");
        assert_eq!(r("2:1-5"), "2/1-5");
        assert_eq!(r("2-1-5"), "2/1-5");
        assert_eq!(r("2:7:7"), "2/7");
        assert_eq!(parse_chapter_and_range("2", "3-3").unwrap().canonical_path(), "2/3");
    }

    #[test]
    fn clamping() {
        let s = VerseSpan::new(1, 300).unwrap();
        assert_eq!(s.clamp_to(286), Ok(VerseSpan { from: 1, to: 286 }));
        assert_eq!(VerseSpan::single(7).unwrap().clamp_to(7), Ok(VerseSpan { from: 7, to: 7 }));
        assert_eq!(
            VerseSpan::single(8).unwrap().clamp_to(7),
            Err(ParseError::VerseOutOfRange { verses_count: 7 })
        );
        assert!(s.contains(1) && s.contains(300) && !s.contains(301));
    }

    #[test]
    fn page_and_juz_bounds() {
        assert!(valid_mushaf_page(1) && valid_mushaf_page(604));
        assert!(!valid_mushaf_page(0) && !valid_mushaf_page(605));
        assert!(valid_juz(1) && valid_juz(30));
        assert!(!valid_juz(0) && !valid_juz(31));
    }

    #[test]
    fn api_pagination() {
        assert_eq!(api_page_of(1, 50), 1);
        assert_eq!(api_page_of(50, 50), 1);
        assert_eq!(api_page_of(51, 50), 2);
        assert_eq!(api_page_of(255, 50), 6);
        assert_eq!(api_page_of(0, 50), 1);
        assert_eq!(api_page_of(5, 0), 5);
        let p = |a, b| api_pages_for(VerseSpan::new(a, b).unwrap(), 50);
        assert_eq!(p(120, 160), 3..=4);
        assert_eq!(p(1, 7), 1..=1);
        assert_eq!(p(255, 255), 6..=6);
        assert_eq!(p(1, 286), 1..=6);
        assert_eq!(p(100, 101), 2..=3);
    }

    #[test]
    fn next_page_capping() {
        assert_eq!(next_page_within(Some(4), Some(4)), Some(4));
        assert_eq!(next_page_within(Some(5), Some(4)), None);
        assert_eq!(next_page_within(None, Some(4)), None);
        assert_eq!(next_page_within(Some(9), None), Some(9));
        assert_eq!(next_page_within(None, None), None);
    }

    #[test]
    fn revelation_labels() {
        assert_eq!(revelation_label("makkah"), "Meccan");
        assert_eq!(revelation_label("Madinah"), "Medinan");
        assert_eq!(revelation_label("?"), "");
    }

    #[test]
    fn juz_mapping_sorts_numerically() {
        let m: std::collections::BTreeMap<String, String> =
            [("10", "1-5"), ("2", "142-252"), ("9", "93-129"), ("bad", "1"), ("3", "x")]
                .into_iter()
                .map(|(a, b)| (a.to_string(), b.to_string()))
                .collect();
        let out = juz_mapping(&m);
        assert_eq!(
            out,
            vec![
                (2, VerseSpan { from: 142, to: 252 }),
                (9, VerseSpan { from: 93, to: 129 }),
                (10, VerseSpan { from: 1, to: 5 }),
            ]
        );
    }

    #[test]
    fn slugs() {
        assert_eq!(slugify("An-Najm"), "an-najm");
        assert_eq!(slugify("Al-Baqarah"), "al-baqarah");
        assert_eq!(slugify("Ali 'Imran"), "ali-imran");
        assert_eq!(slugify("Al-Ma'idah"), "al-maidah");
        assert_eq!(slugify("Al-Fātiĥah"), "al-fatihah");
        assert_eq!(slugify("  Ta-Ha "), "ta-ha");
        assert_eq!(slugify("Al-Mu’minun"), "al-muminun");
        assert!(looks_like_slug("an-najm") && looks_like_slug("Al-Baqarah"));
        assert!(!looks_like_slug("2") && !looks_like_slug("2:255") && !looks_like_slug(""));
    }

    #[test]
    fn slug_matching() {
        let names = [
            (1, "Al-Fatihah", "Al-Fātiĥah"),
            (2, "Al-Baqarah", "Al-Baqarah"),
            (3, "Ali 'Imran", "Āli `Imrān"),
            (5, "Al-Ma'idah", "Al-Mā'idah"),
            (53, "An-Najm", "An-Najm"),
        ];
        let m = |s: &str| match_slug(s, names.iter().copied());
        assert_eq!(m("an-najm"), Some(53));
        assert_eq!(m("An-Najm"), Some(53));
        assert_eq!(m("al-baqarah"), Some(2));
        assert_eq!(m("albaqarah"), Some(2));
        assert_eq!(m("ali-imran"), Some(3));
        assert_eq!(m("al-maidah"), Some(5));
        assert_eq!(m("al-ma-idah"), Some(5));
        assert_eq!(m("al-fatihah"), Some(1));
        assert_eq!(m("al-f%C4%81tihah"), None);
        assert_eq!(m("al-fātiĥah"), Some(1));
        assert_eq!(m("nothing"), None);
        assert_eq!(m("---"), None);
    }

    #[test]
    fn fixture_chapter_slugs_are_unique() {
        let list: crate::api::ChaptersResponse =
            serde_json::from_str(include_str!("../tests/fixtures/chapters.json")).unwrap();
        for c in &list.chapters {
            let slug = slugify(&c.name_simple);
            let names = list.chapters.iter().map(|c| (c.id, c.name_simple.as_str(), c.name_complex.as_str()));
            assert_eq!(match_slug(&slug, names), Some(c.id), "{slug}");
        }
    }
}
