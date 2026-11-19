//! Pure 15-line mushaf layout: loaded, sorted verses → pages → lines of words,
//! with surah header and Bismillah lines inserted before each surah's first line.
//!
//! Nothing is ever dropped: a verse whose words lack line data (or whose word list
//! is broken) becomes a [`Item::Flow`] block at its place in reading order.

use std::rc::Rc;

use crate::api::{Verse, Word};

use crate::glyph;
use crate::integrity;

/// One drawable word (or ayah-end marker) with everything the renderer needs.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct WordCell {
    /// `"chapter:verse:position"`.
    pub location: String,
    pub verse_key: String,
    pub verse_number: u16,
    pub page: Option<u16>,
    pub line: Option<u16>,
    /// The `end` (ayah number) marker.
    pub is_end: bool,
    /// QCF glyph code, only if it belongs to `page`'s font.
    pub glyph: Option<String>,
    /// Unicode text for when the glyph can't be drawn; for the end marker, the ayah number.
    pub text: Option<String>,
    /// `text` is in the QPC Hafs encoding (drawn with the Hafs fallback font).
    pub qpc: bool,
    pub translation: Option<String>,
    pub transliteration: Option<String>,
    /// On a verse's first cell: the verse's word count (the cell is the verse's anchor).
    pub anchor: Option<usize>,
}

/// How a line's words are spread across the line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineAlign {
    /// Full mushaf line: words spread edge to edge.
    Justify,
    /// Centred, as printed (pages 1-2 and some surah endings).
    Center,
    /// Words packed at the line's start (right): the rest of the line isn't shown.
    Start,
    /// Words packed at the line's end (left): the start of the line isn't shown.
    End,
}

/// Why a verse is shown as a flowing block instead of on its mushaf lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlowReason {
    /// Words are fine but line data is missing/invalid.
    NoLayout,
    /// The word list is broken; tokens come from the verse-level text.
    VerseText,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    SurahHeader { chapter: u16 },
    Bismillah { chapter: u16 },
    Line { line: u16, cells: Vec<WordCell>, align: LineAlign },
    Flow { verse_key: String, reason: FlowReason, cells: Vec<WordCell> },
}

/// One mushaf page (or the part of it that is shown).
#[derive(Debug, Clone, PartialEq)]
pub struct PageBlock {
    /// `None` only when no word of the block has a page number.
    pub page: Option<u16>,
    pub items: Vec<Item>,
    /// Verses whose first word (anchor) is in this block, in order.
    pub verse_keys: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Layout {
    /// Shared (`Rc`) so views pass pages around without copying every word.
    pub pages: Vec<Rc<PageBlock>>,
}

/// Whether the shown words may start/end in the middle of a mushaf line
/// (verse-range views), so the first/last line must not be justified.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Trim {
    pub start: bool,
    pub end: bool,
    /// Show only the words printed on this mushaf page (the page view): a verse
    /// that runs over a page turn then shows just its part on this page.
    pub page: Option<u16>,
}

impl Layout {
    pub fn has_bismillah(&self) -> bool {
        self.items().any(|i| matches!(i, Item::Bismillah { .. }))
    }

    fn items(&self) -> impl Iterator<Item = &Item> {
        self.pages.iter().flat_map(|p| p.items.iter())
    }

    /// Distinct pages with glyph cells, each with ALL its glyph text (words and
    /// ayah markers) for the font coverage probe, in order of first appearance.
    pub fn glyph_pages(&self) -> Vec<(u16, String)> {
        let mut out: Vec<(u16, String)> = Vec::new();
        for page in &self.pages {
            let Some(p) = page.page else { continue };
            let glyphs: String = page
                .items
                .iter()
                .flat_map(|i| match i {
                    Item::Line { cells, .. } => cells.as_slice(),
                    _ => &[],
                })
                .filter_map(|c| c.glyph.as_deref())
                .collect();
            if glyphs.is_empty() {
                continue;
            }
            match out.iter_mut().find(|(q, _)| *q == p) {
                Some((_, all)) => all.push_str(&glyphs),
                None => out.push((p, glyphs)),
            }
        }
        out
    }

    /// Total number of word cells (words + markers) placed; for tests and sanity checks.
    #[cfg(test)]
    pub fn cell_count(&self) -> usize {
        self.items()
            .map(|i| match i {
                Item::Line { cells, .. } | Item::Flow { cells, .. } => cells.len(),
                _ => 0,
            })
            .sum()
    }
}

/// Lines printed centred in the Madani mushaf (as on quran.com): all of pages 1
/// and 2, and the last lines of some surahs.
pub fn is_centered_line(page: u16, line: u16) -> bool {
    let lines: &[u16] = match page {
        1 | 2 => return true,
        255 => &[2],
        528 => &[9],
        534 => &[6],
        545 => &[6],
        586 => &[1],
        593 => &[2],
        594 => &[5],
        600 => &[10],
        602 => &[5, 15],
        603 => &[10, 15],
        604 => &[4, 9, 14, 15],
        _ => &[],
    };
    lines.contains(&line)
}

/// Whether a surah is preceded by a separate Bismillah line (all but 1 and 9).
pub fn has_bismillah_line(chapter: u16) -> bool {
    chapter != 1 && chapter != 9
}

fn non_empty(t: Option<&crate::api::WordText>) -> Option<String> {
    t?.text.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(String::from)
}

/// The verse's QPC Hafs text split into one token per word (the end marker's
/// token is the ayah number), if it lines up with the word list. The API only
/// has this text per verse.
fn qpc_tokens(v: &Verse) -> Option<Vec<&str>> {
    let tokens: Vec<&str> = v.qpc_uthmani_hafs.as_deref()?.split_whitespace().collect();
    (tokens.len() == v.words.len()).then_some(tokens)
}

fn word_cell(v: &Verse, w: &Word, layout: Option<(u16, u16)>, qpc: Option<&str>) -> WordCell {
    let (text, qpc) = match (qpc, glyph::unicode_text(w)) {
        (Some(t), _) => (Some(t.to_string()), true),
        (None, Some((t, qpc))) => (Some(t), qpc),
        (None, None) if w.is_end_marker() => (Some(glyph::arabic_number(v.verse_number)), false),
        (None, None) => (None, false),
    };
    WordCell {
        location: format!("{}:{}", v.verse_key, w.position),
        verse_key: v.verse_key.clone(),
        verse_number: v.verse_number,
        page: layout.map(|(p, _)| p),
        line: layout.map(|(_, l)| l),
        is_end: w.is_end_marker(),
        // Glyphs only with a known layout page, and only if the glyph's font page
        // (`v2_page`) is that resolved page: the page's font draws them.
        glyph: layout.and_then(|(page, _)| match w.v2_page {
            Some(font_page) if font_page != page => None,
            _ => glyph::glyph_code(w),
        }),
        text,
        qpc,
        translation: if w.is_end_marker() { None } else { non_empty(w.translation.as_ref()) },
        transliteration: if w.is_end_marker() { None } else { non_empty(w.transliteration.as_ref()) },
        anchor: None,
    }
}

/// `(page, line)` per word (see [`integrity::slot_after`]: an `end` marker without
/// line data, or with a line before its verse's last word, joins that word).
/// `None` if any word can't be placed.
fn resolve_layout(v: &Verse) -> Option<Vec<(u16, u16)>> {
    let mut out: Vec<(u16, u16)> = Vec::with_capacity(v.words.len());
    for w in &v.words {
        out.push(integrity::slot_after(out.last().copied(), w)?);
    }
    Some(out)
}

/// Cells for a verse shown as flowing text.
fn flow_cells(v: &Verse, reason: FlowReason) -> Vec<WordCell> {
    let mut cells: Vec<WordCell> = match reason {
        FlowReason::NoLayout => {
            let qpc = qpc_tokens(v);
            v.words.iter().enumerate().map(|(i, w)| word_cell(v, w, None, qpc.as_ref().map(|t| t[i]))).collect()
        }
        FlowReason::VerseText => {
            let tokens: Vec<&str> = integrity::verse_text(v, "text_uthmani")
                .map(|t| t.split_whitespace().collect())
                .unwrap_or_default();
            let blank =
                WordCell { verse_key: v.verse_key.clone(), verse_number: v.verse_number, ..WordCell::default() };
            let mut cells: Vec<WordCell> = tokens
                .iter()
                .enumerate()
                .map(|(i, t)| WordCell {
                    location: format!("{}:{}", v.verse_key, i + 1),
                    text: Some(t.to_string()),
                    ..blank.clone()
                })
                .collect();
            if cells.is_empty() {
                // No text at all: one visible placeholder rather than nothing.
                cells.push(WordCell { location: format!("{}:1", v.verse_key), ..blank.clone() });
            }
            let n = cells.len();
            cells.push(WordCell {
                location: format!("{}:{}", v.verse_key, n + 1),
                is_end: true,
                text: Some(glyph::arabic_number(v.verse_number)),
                ..blank
            });
            cells
        }
    };
    let count = cells.iter().filter(|c| !c.is_end).count();
    if let Some(first) = cells.first_mut() {
        first.anchor = Some(count);
    }
    cells
}

fn header_items(chapter: u16) -> Vec<Item> {
    let mut items = vec![Item::SurahHeader { chapter }];
    if has_bismillah_line(chapter) {
        items.push(Item::Bismillah { chapter });
    }
    items
}

/// Group verses (sorted, deduplicated) into pages and lines in mushaf order.
pub fn build_layout(verses: &[Verse], trim: Trim) -> Layout {
    let mut pages: Vec<PageBlock> = Vec::new();
    // Current (page, line) being filled; `None` forces a new line item.
    let mut cur: Option<(u16, u16)> = None;

    for v in verses {
        let chapter = v.chapter_number().unwrap_or(0);
        let starts_surah = v.verse_number == 1 && chapter != 0;
        let placeable = integrity::words_structurally_ok(v) && integrity::verse_layout_ok(v);
        let slots = if placeable { resolve_layout(v) } else { None };

        let Some(slots) = slots else {
            // Flowing block, in reading order, on the current page (or the verse's own).
            let reason =
                if integrity::words_structurally_ok(v) { FlowReason::NoLayout } else { FlowReason::VerseText };
            if pages.is_empty() {
                let page = v.page_number.or_else(|| v.words.iter().find_map(|w| w.page_number));
                pages.push(PageBlock { page, items: Vec::new(), verse_keys: Vec::new() });
            }
            let block = pages.last_mut().expect("a page block exists");
            if starts_surah {
                block.items.extend(header_items(chapter));
            }
            block.items.push(Item::Flow { verse_key: v.verse_key.clone(), reason, cells: flow_cells(v, reason) });
            block.verse_keys.push(v.verse_key.clone());
            cur = None;
            continue;
        };

        let word_count = integrity::word_count(v);
        let qpc = qpc_tokens(v);
        let mut header_pending = starts_surah;
        let mut first_shown = true;
        for (i, (w, &(page, line))) in v.words.iter().zip(&slots).enumerate() {
            if trim.page.is_some_and(|only| only != page) {
                continue;
            }
            if pages.last().is_none_or(|b| b.page != Some(page)) {
                pages.push(PageBlock { page: Some(page), items: Vec::new(), verse_keys: Vec::new() });
                cur = None;
            }
            let block = pages.last_mut().expect("a page block exists");
            if header_pending {
                block.items.extend(header_items(chapter));
                header_pending = false;
                cur = None;
            }
            if cur != Some((page, line)) {
                let align = if is_centered_line(page, line) { LineAlign::Center } else { LineAlign::Justify };
                block.items.push(Item::Line { line, cells: Vec::new(), align });
                cur = Some((page, line));
            }
            let mut cell = word_cell(v, w, Some((page, line)), qpc.as_ref().map(|t| t[i]));
            // The verse's anchor is on the first of its words that is shown.
            if first_shown {
                first_shown = false;
                cell.anchor = Some(word_count);
                block.verse_keys.push(v.verse_key.clone());
            }
            if let Some(Item::Line { cells, .. }) = block.items.last_mut() {
                cells.push(cell);
            }
        }
    }

    apply_trim(&mut pages, trim);
    Layout { pages: pages.into_iter().map(Rc::new).collect() }
}

/// Un-justify the first/last line when the view may cut through them.
fn apply_trim(pages: &mut [PageBlock], trim: Trim) {
    let mut lines = pages.iter_mut().flat_map(|p| p.items.iter_mut()).filter_map(|i| match i {
        Item::Line { align, .. } => Some(align),
        _ => None,
    });
    let first = lines.next();
    let last = lines.last();
    let set = |a: &mut LineAlign, partial: LineAlign| {
        *a = match (*a, partial) {
            (LineAlign::Center, _) => LineAlign::Center,
            (LineAlign::Start, LineAlign::End) | (LineAlign::End, LineAlign::Start) => LineAlign::Center,
            _ => partial,
        };
    };
    match (first, last) {
        (Some(f), Some(l)) => {
            if trim.start {
                set(f, LineAlign::End);
            }
            if trim.end {
                set(l, LineAlign::Start);
            }
        }
        (Some(only), None) => {
            if trim.start {
                set(only, LineAlign::End);
            }
            if trim.end {
                set(only, LineAlign::Start);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Verse `key` whose words (last one = end marker) sit on `(page, line)` slots.
    pub(crate) fn verse(key: &str, slots: &[(u16, u16)]) -> Verse {
        let n = slots.len();
        let words: Vec<serde_json::Value> = slots
            .iter()
            .enumerate()
            .map(|(i, (p, l))| {
                let pos = i as u16 + 1;
                let end = i + 1 == n;
                json!({
                    "id": pos, "position": pos, "char_type_name": if end { "end" } else { "word" },
                    "qpc_uthmani_hafs": format!("w{pos}"), "code_v2": format!("g{pos}"),
                    "page_number": p, "line_number": l, "v2_page": p,
                    "translation": { "text": if end { "(1)".to_string() } else { format!("t{pos}") } }
                })
            })
            .collect();
        let num: u16 = key.split(':').nth(1).unwrap().parse().unwrap();
        serde_json::from_value(json!({
            "id": num, "verse_number": num, "verse_key": key, "text_uthmani": "a b c", "words": words
        }))
        .unwrap()
    }

    fn kinds(block: &PageBlock) -> Vec<String> {
        block
            .items
            .iter()
            .map(|i| match i {
                Item::SurahHeader { chapter } => format!("H{chapter}"),
                Item::Bismillah { chapter } => format!("B{chapter}"),
                Item::Line { line, cells, .. } => format!("L{line}x{}", cells.len()),
                Item::Flow { verse_key, .. } => format!("F{verse_key}"),
            })
            .collect()
    }

    #[test]
    fn groups_words_into_pages_and_lines() {
        let vs = vec![
            verse("2:4", &[(2, 14), (2, 14), (2, 15), (2, 15)]),
            verse("2:5", &[(3, 1), (3, 1), (3, 2)]),
        ];
        let l = build_layout(&vs, Trim::default());
        assert_eq!(l.pages.len(), 2);
        assert_eq!(l.pages[0].page, Some(2));
        assert_eq!(kinds(&l.pages[0]), ["L14x2", "L15x2"]);
        assert_eq!(kinds(&l.pages[1]), ["L1x2", "L2x1"]);
        assert_eq!(l.pages[0].verse_keys, ["2:4"]);
        assert_eq!(l.pages[1].verse_keys, ["2:5"]);
        assert_eq!(l.cell_count(), 7);
        // Anchor on each verse's first word, counting non-end words.
        let Item::Line { cells, .. } = &l.pages[0].items[0] else { panic!() };
        assert_eq!(cells[0].anchor, Some(3));
        assert_eq!(cells[0].location, "2:4:1");
        assert_eq!(cells[1].anchor, None);
        assert_eq!((cells[0].page, cells[0].line), (Some(2), Some(14)));
        assert_eq!(cells[0].glyph.as_deref(), Some("g1"));
        assert_eq!(cells[0].text.as_deref(), Some("w1"));
        assert_eq!(cells[0].translation.as_deref(), Some("t1"));
        // Every glyph on each page (words and ayah markers) is handed to the font probe.
        assert_eq!(l.glyph_pages(), vec![(2, "g1g2g3g4".to_string()), (3, "g1g2g3".to_string())]);
    }

    #[test]
    fn verses_sharing_a_line_share_one_line_item() {
        let vs = vec![verse("112:1", &[(604, 2), (604, 2)]), verse("112:2", &[(604, 2), (604, 3)])];
        let l = build_layout(&vs, Trim::default());
        // 112:1 starts the surah: header + Bismillah before its line.
        assert_eq!(kinds(&l.pages[0]), ["H112", "B112", "L2x3", "L3x1"]);
        assert!(l.has_bismillah());
    }

    #[test]
    fn header_and_bismillah_rules() {
        let l = build_layout(&[verse("1:1", &[(1, 2), (1, 2)])], Trim::default());
        assert_eq!(kinds(&l.pages[0]), ["H1", "L2x2"]);
        assert!(!l.has_bismillah());
        let l = build_layout(&[verse("9:1", &[(187, 2), (187, 2)])], Trim::default());
        assert_eq!(kinds(&l.pages[0]), ["H9", "L2x2"]);
        // Mid-page surah start: header goes right before the X:1 line, on that page.
        let vs = vec![verse("2:286", &[(49, 10), (49, 10)]), verse("3:1", &[(50, 1), (50, 1)])];
        let l = build_layout(&vs, Trim::default());
        assert_eq!(kinds(&l.pages[0]), ["L10x2"]);
        assert_eq!(kinds(&l.pages[1]), ["H3", "B3", "L1x2"]);
        let vs = vec![verse("113:5", &[(604, 8), (604, 8)]), verse("114:1", &[(604, 11), (604, 11)])];
        let l = build_layout(&vs, Trim::default());
        assert_eq!(kinds(&l.pages[0]), ["L8x2", "H114", "B114", "L11x2"]);
        // Not verse 1: no header.
        let l = build_layout(&[verse("2:255", &[(42, 1), (42, 1)])], Trim::default());
        assert_eq!(kinds(&l.pages[0]), ["L1x2"]);
    }

    #[test]
    fn missing_layout_becomes_flow_block_in_order() {
        let mut broken = verse("2:5", &[(3, 1), (3, 1), (3, 2)]);
        broken.words[1].line_number = None;
        let vs = vec![verse("2:4", &[(2, 15), (2, 15)]), broken, verse("2:6", &[(3, 2), (3, 2)])];
        let l = build_layout(&vs, Trim::default());
        assert_eq!(l.pages.len(), 2);
        assert_eq!(kinds(&l.pages[0]), ["L15x2", "F2:5"]);
        assert_eq!(kinds(&l.pages[1]), ["L2x2"]);
        assert_eq!(l.pages[0].verse_keys, ["2:4", "2:5"]);
        let Item::Flow { cells, reason, .. } = &l.pages[0].items[1] else { panic!() };
        assert_eq!(*reason, FlowReason::NoLayout);
        assert_eq!(cells.len(), 3);
        assert_eq!(cells[0].anchor, Some(2));
        assert!(cells.iter().all(|c| c.glyph.is_none() && c.page.is_none()));
        // Every word is still there.
        assert_eq!(l.cell_count(), 2 + 3 + 2);
    }

    #[test]
    fn broken_words_fall_back_to_verse_text() {
        let mut v = verse("2:7", &[(3, 4), (3, 4), (3, 4)]);
        v.words.remove(0); // position gap
        let l = build_layout(&[v], Trim::default());
        assert_eq!(kinds(&l.pages[0]), ["F2:7"]);
        assert_eq!(l.pages[0].page, Some(3));
        let Item::Flow { cells, reason, .. } = &l.pages[0].items[0] else { panic!() };
        assert_eq!(*reason, FlowReason::VerseText);
        let texts: Vec<_> = cells.iter().map(|c| c.text.clone().unwrap_or_default()).collect();
        assert_eq!(texts, ["a", "b", "c", "٧"]);
        assert!(cells[3].is_end);
        assert_eq!(cells[0].anchor, Some(3));
        // No text at all: a placeholder word, still anchored.
        let v: Verse =
            serde_json::from_value(json!({"id": 1, "verse_number": 8, "verse_key": "2:8"})).unwrap();
        let l = build_layout(&[v], Trim::default());
        assert_eq!(l.pages[0].page, None);
        let Item::Flow { cells, .. } = &l.pages[0].items[0] else { panic!() };
        assert_eq!(cells.len(), 2);
        assert_eq!((cells[0].text.clone(), cells[0].anchor), (None, Some(1)));
    }

    #[test]
    fn end_marker_without_line_joins_previous_word() {
        let mut v = verse("2:9", &[(3, 5), (3, 6), (3, 6)]);
        v.words[2].line_number = None;
        v.words[2].page_number = None;
        let l = build_layout(&[v], Trim::default());
        assert_eq!(kinds(&l.pages[0]), ["L5x1", "L6x2"]);
    }

    #[test]
    fn end_marker_on_an_earlier_line_joins_its_verse() {
        // As the API has it for 84:21: words on lines 13 and 14, the marker back on 13.
        let v = verse("84:21", &[(589, 13), (589, 14), (589, 14), (589, 13)]);
        assert!(integrity::layout_issues(std::slice::from_ref(&v)).is_empty(), "not a layout problem");
        let l = build_layout(&[v], Trim::default());
        assert_eq!(kinds(&l.pages[0]), ["L13x1", "L14x3"]);
    }

    #[test]
    fn a_page_view_shows_only_that_pages_words() {
        // A verse listed before the page turn, printed after it, then a verse running on to the next page.
        let before = verse("83:34", &[(589, 1), (589, 1)]);
        let across = verse("83:35", &[(589, 1), (589, 2), (590, 1)]);
        let page = Trim { page: Some(589), ..Trim::default() };
        let l = build_layout(&[before.clone(), across.clone()], page);
        assert_eq!(l.pages.len(), 1);
        assert_eq!(kinds(&l.pages[0]), ["L1x3", "L2x1"]);
        assert_eq!(l.pages[0].verse_keys, ["83:34", "83:35"]);
        // On the next page the second verse's anchor is on its first word shown there.
        let next = build_layout(&[before, across], Trim { page: Some(590), ..Trim::default() });
        assert_eq!(kinds(&next.pages[0]), ["L1x1"]);
        let Item::Line { cells, .. } = &next.pages[0].items[0] else { panic!() };
        assert!(cells[0].anchor.is_some());
        assert_eq!(next.pages[0].verse_keys, ["83:35"]);
    }

    #[test]
    fn glyph_dropped_when_font_page_differs() {
        let mut v = verse("2:10", &[(3, 7), (3, 7)]);
        v.words[0].v2_page = Some(4);
        let l = build_layout(&[v], Trim::default());
        let Item::Line { cells, .. } = &l.pages[0].items[0] else { panic!() };
        assert_eq!(cells[0].glyph, None);
        assert_eq!(cells[0].text.as_deref(), Some("w1"));
        assert_eq!(cells[1].glyph.as_deref(), Some("g2"));
    }

    #[test]
    fn alignment_centered_and_trimmed() {
        let l = build_layout(&[verse("1:2", &[(1, 3), (1, 3)])], Trim::default());
        assert!(matches!(l.pages[0].items[0], Item::Line { align: LineAlign::Center, .. }));
        let vs = vec![verse("2:255", &[(42, 4), (42, 5), (42, 9)]), verse("2:256", &[(42, 9), (42, 10)])];
        let l = build_layout(&vs, Trim { start: true, end: true, page: None });
        let aligns: Vec<LineAlign> = l.pages[0]
            .items
            .iter()
            .filter_map(|i| if let Item::Line { align, .. } = i { Some(*align) } else { None })
            .collect();
        assert_eq!(aligns, [LineAlign::End, LineAlign::Justify, LineAlign::Justify, LineAlign::Start]);
        let l = build_layout(&[verse("2:3", &[(2, 8), (2, 8)])], Trim { start: true, end: true, page: None });
        assert!(matches!(l.pages[0].items[0], Item::Line { align: LineAlign::Center, .. }));
        assert!(is_centered_line(604, 15) && !is_centered_line(604, 1) && is_centered_line(2, 1));
    }

    #[test]
    fn verse_level_qpc_text_is_split_per_word() {
        let mut v = verse("2:4", &[(2, 14), (2, 14), (2, 15)]);
        for w in &mut v.words {
            w.qpc_uthmani_hafs = None;
            w.text_uthmani = Some("u".into());
        }
        v.qpc_uthmani_hafs = Some("بِسۡمِ ٱللَّهِ\u{a0}٤".into());
        let l = build_layout(std::slice::from_ref(&v), Trim::default());
        let cells: Vec<&WordCell> = l
            .pages
            .iter()
            .flat_map(|p| &p.items)
            .flat_map(|i| if let Item::Line { cells, .. } = i { cells.as_slice() } else { &[] })
            .collect();
        let texts: Vec<_> = cells.iter().map(|c| (c.text.as_deref().unwrap(), c.qpc)).collect();
        assert_eq!(texts, [("بِسۡمِ", true), ("ٱللَّهِ", true), ("٤", true)]);
        // Token count differs from the word count: plain Uthmani text, plain ayah number.
        v.qpc_uthmani_hafs = Some("بِسۡمِ ٤".into());
        v.words[2].text_uthmani = None;
        let l = build_layout(&[v], Trim::default());
        let Item::Line { cells, .. } = &l.pages[0].items[1] else { panic!() };
        assert_eq!((cells[0].text.as_deref(), cells[0].qpc), (Some("٤"), false));
        let Item::Line { cells, .. } = &l.pages[0].items[0] else { panic!() };
        assert_eq!((cells[0].text.as_deref(), cells[0].qpc), (Some("u"), false));
    }

    #[test]
    fn empty_input() {
        assert_eq!(build_layout(&[], Trim::default()), Layout::default());
        assert!(Layout::default().glyph_pages().is_empty());
    }

    #[test]
    fn end_marker_glyph_needs_its_resolved_page_font() {
        // End marker without its own layout takes the previous word's slot (page 2);
        // its glyph says it belongs to page 3's font, so it must be drawn as text.
        let mut v = verse("2:4", &[(2, 14), (2, 14)]);
        let end = v.words.last_mut().unwrap();
        end.page_number = None;
        end.line_number = None;
        end.v2_page = Some(3);
        let l = build_layout(&[v], Trim::default());
        let Item::Line { cells, .. } = &l.pages[0].items[0] else { panic!() };
        let marker = cells.iter().find(|c| c.is_end).unwrap();
        assert_eq!(marker.page, Some(2));
        assert_eq!(marker.glyph, None, "glyph from another page's font must not be used");
        assert!(cells.iter().any(|c| !c.is_end && c.glyph.is_some()));
    }
}
