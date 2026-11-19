//! What the app shows — a list or a reader view — and the reading position in
//! it, written as a location like a quran.com URL: the path is the view
//! (`/`, `/juz`, `/2`, `/2/1-5`, `/juz/30`, `/page/22`) and the hash is the
//! word that starts the top line (`/2#2:142`, `/juz/2#2:282:45`). The app saves
//! its location and starts there next time.

use std::fmt;

use crate::api::{Chapter, Juz};
use crate::feed::VerseSource;
use crate::lists::{find_chapter, juz_of};
use crate::mushaf_layout::Trim;
use crate::parse::{self, CHAPTER_COUNT, JUZ_COUNT, MUSHAF_PAGES, VerseSpan};
use crate::reference;

/// How the Quran is divided for reading and browsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Surah,
    Juz,
    Page,
}

impl Mode {
    pub const ALL: [Mode; 3] = [Mode::Surah, Mode::Juz, Mode::Page];

    pub fn label(self) -> &'static str {
        match self {
            Mode::Surah => "Surah",
            Mode::Juz => "Juz",
            Mode::Page => "Page",
        }
    }

    fn name(self) -> &'static str {
        match self {
            Mode::Surah => "surah",
            Mode::Juz => "juz",
            Mode::Page => "page",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    SurahList,
    JuzList,
    /// A surah, or with `span` a range of its verses.
    Surah { chapter: u16, span: Option<VerseSpan> },
    Juz(u8),
    Page(u16),
}

impl View {
    pub fn surah(chapter: u16) -> Self {
        View::Surah { chapter, span: None }
    }

    /// The list of surahs or of ajzāʾ; pages have none.
    pub fn index(mode: Mode) -> Option<Self> {
        match mode {
            Mode::Surah => Some(View::SurahList),
            Mode::Juz => Some(View::JuzList),
            Mode::Page => None,
        }
    }

    pub fn mode(&self) -> Mode {
        match self {
            View::SurahList | View::Surah { .. } => Mode::Surah,
            View::JuzList | View::Juz(_) => Mode::Juz,
            View::Page(_) => Mode::Page,
        }
    }

    /// The verses a reader view loads; `None` for the lists.
    pub fn source(&self) -> Option<VerseSource> {
        match *self {
            View::SurahList | View::JuzList => None,
            View::Surah { chapter, span } => Some(VerseSource::Chapter { chapter, span }),
            View::Juz(juz) => Some(VerseSource::Juz(juz)),
            View::Page(page) => Some(VerseSource::Page(page)),
        }
    }

    /// The view if it exists: surah 1-114, juz 1-30, page 1-604; a verse range
    /// must start inside its surah and is cut at the surah's end.
    pub fn checked(self) -> Result<Self, String> {
        match self {
            View::SurahList | View::JuzList => Ok(self),
            View::Page(page) if parse::valid_mushaf_page(page) => Ok(self),
            View::Page(page) => Err(format!("Page {page} does not exist; the mushaf has pages 1 to {MUSHAF_PAGES}.")),
            View::Juz(juz) if parse::valid_juz(juz) => Ok(self),
            View::Juz(juz) => Err(format!("Juz {juz} does not exist; there are {JUZ_COUNT} ajzāʾ.")),
            View::Surah { chapter, span } => {
                let count = reference::verses_in(chapter)
                    .ok_or_else(|| format!("Surah {chapter} does not exist; there are {CHAPTER_COUNT} surahs."))?;
                let span = span.map(|s| s.clamp_to(count)).transpose().map_err(|e| e.to_string())?;
                Ok(View::Surah { chapter, span })
            }
        }
    }

    /// "Surah Al-Baqarah", "Juz 2" or "Page 22": the heading of a reader view.
    pub fn title(&self, chapters: &[Chapter]) -> Option<String> {
        match *self {
            View::SurahList | View::JuzList => None,
            View::Page(page) => Some(format!("Page {page}")),
            View::Juz(juz) => Some(format!("Juz {juz}")),
            View::Surah { chapter, .. } => Some(surah_title(chapters, chapter)),
        }
    }

    /// Every verse that must be present once the view has loaded completely.
    /// (A juz needs the juz list; until it is there, nothing is expected.)
    pub fn expected(&self, juzs: &[Juz]) -> Vec<(u16, VerseSpan)> {
        match *self {
            View::SurahList | View::JuzList => Vec::new(),
            View::Surah { chapter, span } => {
                let full = reference::verses_in(chapter).map(|n| VerseSpan { from: 1, to: n.max(1) });
                span.or(full).map(|s| vec![(chapter, s)]).unwrap_or_default()
            }
            View::Page(page) => reference::verses_on_page(page),
            View::Juz(juz) => juzs
                .iter()
                .find(|j| j.juz_number == juz)
                .map(|j| parse::juz_mapping(&j.verse_mapping))
                .unwrap_or_default(),
        }
    }

    /// A range may start/end mid-line: those lines aren't stretched edge to edge.
    pub fn trim(&self) -> Trim {
        match *self {
            View::Surah { chapter, span: Some(s) } => {
                let count = reference::verses_in(chapter).unwrap_or(u16::MAX);
                Trim { start: s.from > 1, end: s.to < count, page: None }
            }
            // A page view shows the words printed on its page, and no others.
            View::Page(page) => Trim { page: Some(page), ..Trim::default() },
            _ => Trim::default(),
        }
    }

    /// The same view opened at a reading position.
    pub fn at(self, at: Option<WordRef>) -> Location {
        Location { view: self, at: at.filter(|_| self.source().is_some()) }
    }
}

fn surah_title(chapters: &[Chapter], chapter: u16) -> String {
    match find_chapter(chapters, chapter) {
        Some(c) => format!("Surah {}", c.name_simple),
        None => format!("Surah {chapter}"),
    }
}

impl fmt::Display for View {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            View::SurahList => write!(f, "/"),
            View::JuzList => write!(f, "/juz"),
            View::Surah { chapter, span: None } => write!(f, "/{chapter}"),
            View::Surah { chapter, span: Some(s) } => write!(f, "/{chapter}/{}", s.segment()),
            View::Juz(juz) => write!(f, "/juz/{juz}"),
            View::Page(page) => write!(f, "/page/{page}"),
        }
    }
}

/// A word of the Quran: `2:142:1` is the first word of verse 2:142.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WordRef {
    pub chapter: u16,
    pub verse: u16,
    pub word: u16,
}

impl WordRef {
    /// `"2:142"` (the verse's first word) or `"2:142:7"`; `None` for anything else.
    pub fn parse(s: &str) -> Option<Self> {
        let mut parts = s.split(':').map(|n| n.parse::<u16>().ok().filter(|n| *n > 0));
        let (chapter, verse) = (parts.next()??, parts.next()??);
        let word = parts.next().unwrap_or(Some(1))?;
        parts.next().is_none().then_some(WordRef { chapter, verse, word })
    }

    pub fn verse_key(&self) -> String {
        format!("{}:{}", self.chapter, self.verse)
    }

    /// `"chapter:verse:word"`, as in a word's `location`.
    pub fn location(&self) -> String {
        format!("{}:{}:{}", self.chapter, self.verse, self.word)
    }
}

impl fmt::Display for WordRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.chapter, self.verse)?;
        if self.word > 1 {
            write!(f, ":{}", self.word)?;
        }
        Ok(())
    }
}

/// A view and the reading position in it (`None` at the top of the view).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Location {
    pub view: View,
    pub at: Option<WordRef>,
}

impl Location {
    /// Parse `/2#2:142`, `/juz/30`, `/page`, `/2:255`... `None` if the path is no
    /// view or the view doesn't exist. A hash that isn't a position is ignored.
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        let (path, hash) = s.split_once('#').unwrap_or((s, ""));
        let segments: Vec<&str> = path.strip_prefix('/')?.split('/').filter(|p| !p.is_empty()).collect();
        let view = match segments.as_slice() {
            [] => View::SurahList,
            ["juz"] => View::JuzList,
            // There is no list of pages: `/page` opens the first one.
            ["page"] => View::Page(1),
            ["juz", n] => View::Juz(n.parse().ok()?),
            ["page", n] => View::Page(n.parse().ok()?),
            [chapter] => {
                let r = parse::parse_chapter_segment(chapter).ok()?;
                View::Surah { chapter: r.chapter, span: r.span }
            }
            [chapter, range] => {
                let r = parse::parse_chapter_and_range(chapter, range).ok()?;
                View::Surah { chapter: r.chapter, span: r.span }
            }
            _ => return None,
        };
        Some(view.checked().ok()?.at(WordRef::parse(hash)))
    }
}

impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.view.fmt(f)?;
        match self.at {
            Some(at) => write!(f, "#{at}"),
            None => Ok(()),
        }
    }
}

/// What a URL of the web app names: a view at a reading position, or the
/// results of a search (`/search?q=light%20%26%20dark`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Place {
    At(Location),
    Search(String),
}

impl Place {
    /// Parse a path with its query and hash, as the address bar has it below
    /// the site's base. `None` if it names nothing.
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        if let Some(rest) = s.strip_prefix("/search") {
            let query = rest.split('#').next().unwrap_or("");
            let query = match query.strip_prefix('?') {
                Some(pairs) => pairs.split('&').find_map(|pair| pair.strip_prefix("q=")).unwrap_or(""),
                None if query.is_empty() || query == "/" => "",
                None => return None,
            };
            return Some(Place::Search(percent_decode(query)));
        }
        Location::parse(s).map(Place::At)
    }
}

impl fmt::Display for Place {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Place::At(location) => location.fmt(f),
            Place::Search(query) => write!(f, "/search?q={}", crate::api::encode(query)),
        }
    }
}

/// Decode `%XX` escapes and `+` (a space, in a query); bytes that are not
/// UTF-8 afterwards are replaced, never an error.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |at: usize| bytes.get(at).and_then(|b| (*b as char).to_digit(16));
        match (bytes[i], hex(i + 1), hex(i + 2)) {
            (b'%', Some(high), Some(low)) => {
                out.push((high * 16 + low) as u8);
                i += 3;
            }
            (b'+', ..) => {
                out.push(b' ');
                i += 1;
            }
            (byte, ..) => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Where the reader is: the word that starts the top line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Position {
    pub at: WordRef,
    /// The mushaf page of that line (0 if unknown).
    pub page: u16,
    /// The view is scrolled to its very top.
    pub top: bool,
}

/// What the toggle knows about the reader: the view on screen, the place being
/// read (if any), the last place read, and the lists it needs for names.
pub struct Reading<'a> {
    /// `None` on a screen that belongs to no mode (search results).
    pub view: Option<View>,
    pub here: Option<Position>,
    pub last: Option<Position>,
    pub juzs: &'a [Juz],
    pub chapters: &'a [Chapter],
}

impl Reading<'_> {
    /// The view of `mode` that shows the place `p`, opened at it.
    fn view_of(&self, mode: Mode, p: Position) -> Option<Location> {
        let view = match mode {
            Mode::Surah => View::surah(p.at.chapter),
            Mode::Juz => View::Juz(juz_of(self.juzs, p.at.chapter, p.at.verse)?),
            Mode::Page => View::Page(Some(p.page).filter(|page| *page > 0)?),
        };
        Some(view.at(Some(p.at)))
    }

    /// Where the toggle's button for `mode` leads, and its tooltip.
    ///
    /// - The current mode's button flips between its list and reading: from a
    ///   view to the list, from the list back to the last place read (or to the
    ///   beginning, if nothing was read yet). Pages have no list, so there the
    ///   button stays where it is.
    /// - Another mode's button opens the place being read in that mode's view;
    ///   when nothing is being read, that mode's list, or for pages the last
    ///   place read.
    pub fn target(&self, mode: Mode) -> (Location, String) {
        let name = mode.name();
        let title = |to: &View| to.title(self.chapters).unwrap_or_else(|| name.to_string());
        let list = View::index(mode).map(|list| (list.at(None), format!("Show the {name} list")));
        let resume = || match self.last.and_then(|p| self.view_of(mode, p)) {
            Some(to) => {
                let tip = format!("Back to {}", title(&to.view));
                (to, tip)
            }
            None => {
                let to = match mode {
                    Mode::Surah => View::surah(1),
                    Mode::Juz => View::Juz(1),
                    Mode::Page => View::Page(1),
                };
                let tip = format!("Open {}", title(&to));
                (to.at(None), tip)
            }
        };
        if self.view.map(|v| v.mode()) != Some(mode) {
            return match self.here.and_then(|p| self.view_of(mode, p)) {
                Some(to) => (to, format!("Read here by {name}")),
                None => list.unwrap_or_else(resume),
            };
        }
        match (list, self.view) {
            (Some((list, tip)), view) if view != Some(list.view) => (list, tip),
            (Some(_), _) => resume(),
            (None, Some(view)) => (view.at(None), format!("Reading by {name}")),
            (None, None) => resume(),
        }
    }
}

/// Previous / next of a reader view. The Quran reads right to left, so the app
/// shows "next" on the left and "previous" on the right.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Neighbours {
    pub prev: Option<(View, String)>,
    pub next: Option<(View, String)>,
    /// For a verse range: the whole surah.
    pub middle: Option<(View, String)>,
}

impl Neighbours {
    /// Neighbouring pages and ajzāʾ; for a surah, neighbouring surahs — or for a
    /// single verse, neighbouring verses — plus a "full surah" link for ranges.
    pub fn of(view: &View, chapters: &[Chapter]) -> Self {
        match *view {
            View::SurahList | View::JuzList => Self::default(),
            View::Page(page) => Self {
                prev: (page > 1).then(|| (View::Page(page - 1), format!("Page {}", page - 1))),
                next: (page < MUSHAF_PAGES).then(|| (View::Page(page + 1), format!("Page {}", page + 1))),
                middle: None,
            },
            View::Juz(juz) => Self {
                prev: (juz > 1).then(|| (View::Juz(juz - 1), format!("Juz {}", juz - 1))),
                next: (juz < JUZ_COUNT).then(|| (View::Juz(juz + 1), format!("Juz {}", juz + 1))),
                middle: None,
            },
            View::Surah { chapter, span } => {
                let name = find_chapter(chapters, chapter).map_or(chapter.to_string(), |c| c.name_simple.clone());
                let count = reference::verses_in(chapter).unwrap_or(0);
                let middle = span.map(|_| (View::surah(chapter), format!("All of {name}")));
                if let Some(s) = span.filter(VerseSpan::is_single) {
                    let verse = |n: u16| {
                        let span = VerseSpan::single(n).ok();
                        (View::Surah { chapter, span }, format!("Verse {n}"))
                    };
                    return Self {
                        prev: (s.from > 1).then(|| verse(s.from - 1)),
                        next: (s.from < count).then(|| verse(s.from + 1)),
                        middle,
                    };
                }
                let surah = |n: u16| (View::surah(n), surah_title(chapters, n));
                Self {
                    prev: (chapter > 1).then(|| surah(chapter - 1)),
                    next: (chapter < CHAPTER_COUNT).then(|| surah(chapter + 1)),
                    middle,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn juz(n: u8, chapter: u16, range: &str) -> Juz {
        serde_json::from_value(serde_json::json!({
            "id": n, "juz_number": n, "verse_mapping": { chapter.to_string(): range }
        }))
        .unwrap()
    }

    fn span(from: u16, to: u16) -> Option<VerseSpan> {
        Some(VerseSpan { from, to })
    }

    #[test]
    fn reading_position_lives_in_the_hash() {
        let at = Some(WordRef { chapter: 2, verse: 142, word: 1 });
        for (url, location) in [
            ("/2#2:142", View::surah(2).at(at)),
            ("/juz/2#2:142", View::Juz(2).at(at)),
            ("/page/22#2:142", View::Page(22).at(at)),
            ("/2/140-150#2:142", View::Surah { chapter: 2, span: span(140, 150) }.at(at)),
            ("/2/255", View::Surah { chapter: 2, span: span(255, 255) }.at(None)),
            ("/2", View::surah(2).at(None)),
            ("/juz", View::JuzList.at(None)),
            ("/", View::SurahList.at(None)),
        ] {
            assert_eq!(location.to_string(), url);
            assert_eq!(Location::parse(url), Some(location), "{url}");
        }
        let mid = Some(WordRef { chapter: 2, verse: 282, word: 45 });
        assert_eq!(View::surah(2).at(mid).to_string(), "/2#2:282:45");
        // A hash that isn't a position is ignored, never an error.
        for url in ["/2#nonsense", "/2#0:1", "/2#2:5:1:9", "/2#"] {
            assert_eq!(Location::parse(url), Some(View::surah(2).at(None)), "{url}");
        }
        // Lists have no reading position.
        assert_eq!(View::SurahList.at(at), Location { view: View::SurahList, at: None });
        assert_eq!(Location::parse("/juz#2:142"), Some(View::JuzList.at(None)));
    }

    #[test]
    fn locations_that_are_no_view() {
        assert_eq!(Location::parse("/page"), Some(View::Page(1).at(None)), "/page opens the first page");
        // Legacy key forms still open the verse.
        assert_eq!(Location::parse("/2:255"), Some(View::Surah { chapter: 2, span: span(255, 255) }.at(None)));
        // A range is cut at the end of its surah.
        assert_eq!(Location::parse("/1/5-99"), Some(View::Surah { chapter: 1, span: span(5, 7) }.at(None)));
        for bad in ["", "2", "/0", "/115", "/page/0", "/page/605", "/juz/31", "/juz/x", "/1/8", "/a/b/c", "/search"] {
            assert_eq!(Location::parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn views_know_their_mode_title_and_source() {
        assert_eq!(View::surah(2).mode(), Mode::Surah);
        assert_eq!(View::Juz(2).mode(), Mode::Juz);
        assert_eq!(View::Page(3).mode(), Mode::Page);
        assert_eq!((View::index(Mode::Juz), View::index(Mode::Page)), (Some(View::JuzList), None));
        assert_eq!(View::Page(22).title(&[]).as_deref(), Some("Page 22"));
        assert_eq!(View::Juz(2).title(&[]).as_deref(), Some("Juz 2"));
        assert_eq!(View::surah(2).title(&[]).as_deref(), Some("Surah 2"));
        assert_eq!(View::JuzList.title(&[]), None);
        assert_eq!(View::Page(3).source(), Some(VerseSource::Page(3)));
        assert_eq!(View::SurahList.source(), None);
        assert!(View::Page(605).checked().is_err() && View::Juz(0).checked().is_err());
        assert!(View::surah(115).checked().is_err());
    }

    #[test]
    fn expected_verses_and_trim() {
        assert_eq!(View::surah(1).expected(&[]), [(1, VerseSpan { from: 1, to: 7 })]);
        let range = View::Surah { chapter: 2, span: span(140, 150) };
        assert_eq!(range.expected(&[]), [(2, VerseSpan { from: 140, to: 150 })]);
        assert_eq!(range.trim(), Trim { start: true, end: true, page: None });
        assert_eq!(View::Page(589).trim(), Trim { page: Some(589), ..Trim::default() });
        assert_eq!(View::Surah { chapter: 1, span: span(1, 7) }.trim(), Trim::default());
        assert_eq!(View::surah(2).trim(), Trim::default());
        // Page 1 is Al-Fatihah; page 589 begins with 83:34, which the API lists under 588.
        assert_eq!(View::Page(1).expected(&[]), [(1, VerseSpan { from: 1, to: 7 })]);
        assert_eq!(View::Page(589).expected(&[])[0], (83, VerseSpan { from: 34, to: 36 }));
        let juzs = [juz(2, 2, "142-252")];
        assert_eq!(View::Juz(2).expected(&juzs), [(2, VerseSpan { from: 142, to: 252 })]);
        assert!(View::Juz(3).expected(&juzs).is_empty());
    }

    #[test]
    fn toggle_flips_between_list_and_reading() {
        let juzs = [juz(1, 2, "1-141"), juz(2, 2, "142-252")];
        let at = WordRef { chapter: 2, verse: 142, word: 1 };
        let place = Some(Position { at, page: 22, top: false });
        let reading = |view: View, here, last| {
            let r = Reading { view: Some(view), here, last, juzs: &juzs, chapters: &[] };
            Mode::ALL.map(|m| r.target(m))
        };
        let tip = |to: Location, tip: &str| (to, tip.to_string());

        // Reading juz 2: Juz shows the list, the others open the same line in their view.
        assert_eq!(
            reading(View::Juz(2), place, place),
            [
                tip(View::surah(2).at(Some(at)), "Read here by surah"),
                tip(View::JuzList.at(None), "Show the juz list"),
                tip(View::Page(22).at(Some(at)), "Read here by page"),
            ]
        );
        // Pages have no list: while reading by page, the Page button stays put.
        assert_eq!(reading(View::Page(22), place, place)[2], tip(View::Page(22).at(None), "Reading by page"));
        // On the juz list afterwards: Juz and Page lead back, Surah to its list.
        assert_eq!(
            reading(View::JuzList, None, place),
            [
                tip(View::SurahList.at(None), "Show the surah list"),
                tip(View::Juz(2).at(Some(at)), "Back to Juz 2"),
                tip(View::Page(22).at(Some(at)), "Back to Page 22"),
            ]
        );
        // Nothing read yet: the buttons that lead to reading start at the beginning.
        assert_eq!(
            reading(View::SurahList, None, None),
            [
                tip(View::surah(1).at(None), "Open Surah 1"),
                tip(View::JuzList.at(None), "Show the juz list"),
                tip(View::Page(1).at(None), "Open Page 1"),
            ]
        );
        // Search results belong to no mode: lists, and for pages the way back.
        let r = Reading { view: None, here: None, last: place, juzs: &juzs, chapters: &[] };
        assert_eq!(r.target(Mode::Juz), tip(View::JuzList.at(None), "Show the juz list"));
        assert_eq!(r.target(Mode::Page), tip(View::Page(22).at(Some(at)), "Back to Page 22"));
        // A view whose position isn't known yet (still loading) offers the other list.
        assert_eq!(reading(View::Juz(2), None, None)[0], tip(View::SurahList.at(None), "Show the surah list"));
    }

    #[test]
    fn places_round_trip_through_urls() {
        for query in ["light & dark", "mercy", "50% & more #1 a+b", "نور"] {
            let place = Place::Search(query.into());
            let url = place.to_string();
            assert!(url.starts_with("/search?q=") && !url.contains(' ') && !url.contains('#'), "{url}");
            assert_eq!(Place::parse(&url), Some(place), "{url}");
        }
        assert_eq!(Place::parse("/search?q=light%20%26%20dark"), Some(Place::Search("light & dark".into())));
        assert_eq!(Place::parse("/search?q=a+b&x=1#top"), Some(Place::Search("a b".into())));
        assert_eq!(Place::parse("/search"), Some(Place::Search(String::new())));
        assert_eq!(Place::parse("/search?q=%zz%4"), Some(Place::Search("%zz%4".into())), "a stray % is kept");
        // Everything else is a view, as in `Location`.
        let at = Some(WordRef { chapter: 2, verse: 142, word: 1 });
        assert_eq!(Place::parse("/juz/2#2:142"), Some(Place::At(View::Juz(2).at(at))));
        assert_eq!(Place::At(View::Page(589).at(None)).to_string(), "/page/589");
        assert_eq!((Place::parse("/searching"), Place::parse("/nowhere/at/all")), (None, None));
    }

    #[test]
    fn neighbours() {
        let n = Neighbours::of(&View::Page(1), &[]);
        assert_eq!((n.prev, n.next), (None, Some((View::Page(2), "Page 2".into()))));
        let n = Neighbours::of(&View::Page(604), &[]);
        assert_eq!((n.prev, n.next), (Some((View::Page(603), "Page 603".into())), None));
        let n = Neighbours::of(&View::Juz(2), &[]);
        assert_eq!((n.prev, n.next), (Some((View::Juz(1), "Juz 1".into())), Some((View::Juz(3), "Juz 3".into()))));
        let n = Neighbours::of(&View::surah(114), &[]);
        assert_eq!((n.prev, n.next, n.middle), (Some((View::surah(113), "Surah 113".into())), None, None));
        // A single verse steps verse by verse and offers the whole surah.
        let verse = |v| View::Surah { chapter: 1, span: span(v, v) };
        let n = Neighbours::of(&verse(7), &[]);
        assert_eq!((n.prev, n.next), (Some((verse(6), "Verse 6".into())), None));
        assert_eq!(n.middle, Some((View::surah(1), "All of 1".into())));
        // A range steps surah by surah.
        let n = Neighbours::of(&View::Surah { chapter: 2, span: span(1, 5) }, &[]);
        assert_eq!(n.next, Some((View::surah(3), "Surah 3".into())));
        assert_eq!(Neighbours::of(&View::JuzList, &[]), Neighbours::default());
    }
}
