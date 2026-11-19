//! Geometry of one mushaf page: every line's words placed right to left on the
//! shared line width, with their meanings underneath. Lines are never wrapped
//! or cut; a full line is justified edge to edge.

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use egui::{Align, Color32, FontId, Galley, Vec2, text::LayoutJob};
use quran_core::api::Verse;
use quran_core::mushaf_layout::{FlowReason, Item, LineAlign, PageBlock, WordCell};
use quran_core::settings::MUSHAF_LINE;

use crate::sheet::Sprite;

/// English meaning of the Bismillah when no translation is selected.
pub const BISMILLAH_MEANING: &str = "In the Name of Allah—the Most Compassionate, Most Merciful";

/// Space above a page's first line (marks on tall letters rise above the line box), in em.
const PAGE_PAD_TOP: f32 = 0.4;
/// Margin above a page, and the page number pill with its margins and the rule under the page.
const PAGE_MARGIN_TOP: f32 = 16.0;
pub const PILL_HEIGHT: f32 = 22.0;
const PILL_MARGIN_TOP: f32 = 8.0;
const PAGE_PAD_BOTTOM: f32 = 12.0;
const PAGE_MARGIN_BOTTOM: f32 = 28.0;

/// Everything a page's geometry depends on besides its words. Lengths in points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Metrics {
    /// The shared scale: 1, or less when the widest line would not fit.
    pub fit: f32,
    /// Size of a glyph em (already scaled by `fit`).
    pub em: f32,
    /// The one line width every page uses.
    pub line_width: f32,
    /// Smallest gap between words of a justified line.
    pub word_gap: f32,
    /// Gap between words of a centred or partial line.
    pub loose_gap: f32,
    /// Space under a line (below its meanings).
    pub line_gap: f32,
    /// Space between a word and its meaning.
    pub meaning_gap: f32,
    pub meaning_font: f32,
    pub meaning_max_width: f32,
    /// Something is shown under the words (meaning, transliteration or both).
    pub meanings: bool,
    /// Which of the two a word shows — under it, or with `tooltip` when hovered.
    pub translation: bool,
    pub transliteration: bool,
    pub tooltip: bool,
    pub muted: Color32,
}

impl Metrics {
    pub fn line_height(&self) -> f32 {
        self.em * MUSHAF_LINE
    }

    /// Height of everything on a page that isn't a line.
    fn chrome(&self) -> f32 {
        PAGE_MARGIN_TOP + self.em * PAGE_PAD_TOP + PILL_MARGIN_TOP + PILL_HEIGHT + PAGE_PAD_BOTTOM + 1.0 + PAGE_MARGIN_BOTTOM
    }

    /// A page's height before it has been laid out.
    pub fn estimate(&self, block: &PageBlock) -> f32 {
        let meaning = if self.meanings { self.meaning_gap + self.meaning_font * 1.25 * 2.2 } else { 0.0 };
        let rows: f32 = block
            .items
            .iter()
            .map(|item| match item {
                Item::SurahHeader { .. } => self.line_height() + self.line_gap,
                Item::Bismillah { .. } => self.line_height() + self.meaning_gap + self.meaning_font * 1.6 + self.line_gap,
                Item::Line { .. } => self.line_height() + meaning + self.line_gap,
                Item::Flow { cells, .. } => (cells.len() as f32 / 8.0).ceil() * (self.line_height() + meaning) + 40.0,
            })
            .sum();
        self.chrome() + rows
    }

    pub fn hash<H: Hasher>(&self, state: &mut H) {
        for v in [
            self.fit,
            self.em,
            self.line_width,
            self.word_gap,
            self.loose_gap,
            self.line_gap,
            self.meaning_gap,
            self.meaning_font,
            self.meaning_max_width,
        ] {
            v.to_bits().hash(state);
        }
        (self.meanings, self.translation, self.transliteration, self.tooltip, self.muted).hash(state);
    }
}

/// Sprite key of a word cell.
pub fn word_key(location: &str) -> u64 {
    key(&("w", location))
}

/// Sprite key of a label of a surah banner or Bismillah line.
pub fn label_key(kind: &str, chapter: u16, part: usize) -> u64 {
    key(&(kind, chapter, part))
}

fn key(value: &impl Hash) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

/// A word or ayah marker in its place.
pub struct Word {
    pub key: u64,
    /// Left edge of the word's box from the line's left edge, and its width.
    pub x: f32,
    pub width: f32,
    /// Top of the word's line from the row's top (non-zero in flowing blocks).
    pub y: f32,
    /// Size of the meaning under it. (Only sizes are kept: egui's laid-out
    /// text is good for one frame, so the meaning is laid out again when painted.)
    pub meaning: Option<Vec2>,
    /// `(item, cell)` in the page block; `None` for the Bismillah's words.
    pub cell: Option<(usize, usize)>,
    /// How it is drawn, as decided when it was placed.
    pub mode: Mode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Glyph,
    /// Shaped fallback text; `ringed`: an ayah number without its medallion, drawn in a ring.
    Text { ringed: bool },
    /// No font yet: a marked box where the word will be.
    Placeholder,
    /// Neither glyph nor text exists for the word: `⟨?⟩`.
    Missing,
}

pub enum RowKind {
    Banner { chapter: u16 },
    Bismillah { words: Vec<Word>, meaning: String },
    Line { words: Vec<Word> },
    Flow { note: String, words: Vec<Word> },
}

pub struct Row {
    /// Index of the row's item in the page block.
    pub item: usize,
    /// Top from the page's top, and height including the space under it.
    pub y: f32,
    pub height: f32,
    pub kind: RowKind,
}

impl Row {
    pub fn words(&self) -> &[Word] {
        match &self.kind {
            RowKind::Banner { .. } => &[],
            RowKind::Bismillah { words, .. } | RowKind::Line { words } | RowKind::Flow { words, .. } => words,
        }
    }

    /// A line of the mushaf text (the reading position is the first word of one).
    pub fn is_text(&self) -> bool {
        matches!(self.kind, RowKind::Line { .. } | RowKind::Flow { .. })
    }
}

pub struct Placed {
    pub stamp: u64,
    pub rows: Vec<Row>,
    pub height: f32,
    /// Top of the page number pill.
    pub pill_y: f32,
    /// Width the widest line needs, when every word of the page is drawn the
    /// way it will stay (glyphs, or text where no glyph can come).
    pub natural: Option<f32>,
    /// Words placed, to be checked against the page's word count.
    pub word_count: usize,
}

/// What the placement of a page reads.
pub struct Input<'a> {
    pub block: &'a PageBlock,
    pub metrics: &'a Metrics,
    pub sprites: Option<&'a std::collections::HashMap<u64, Sprite>>,
    /// Sprites whose glyph mode is final: the page font is loaded, or failed for good.
    pub settled: bool,
    pub bismillah: Option<&'a Verse>,
    /// egui's text layout (for the meanings).
    pub layout: &'a mut dyn FnMut(LayoutJob) -> Arc<Galley>,
}

fn job(text: &str, size: f32, max_width: f32, line_height: f32, halign: Align, color: Color32) -> LayoutJob {
    let mut job = LayoutJob::simple(text.to_owned(), FontId::proportional(size), color, max_width);
    job.halign = halign;
    job.sections[0].format.line_height = Some(size * line_height);
    job
}

/// What a word says about itself: its transliteration and its meaning, as far
/// as the settings ask for them. An ayah marker shows its number where meanings are shown.
pub fn word_notes(cell: &WordCell, m: &Metrics) -> (Option<String>, Option<String>) {
    if cell.is_end {
        return (None, m.translation.then(|| format!("({})", cell.verse_number)));
    }
    (cell.transliteration.clone().filter(|_| m.transliteration), cell.translation.clone().filter(|_| m.translation))
}

/// What is under a word: small, muted, centred, wrapped at its widest; the
/// transliteration (in italics) above the meaning.
pub fn meaning_job(cell: &WordCell, m: &Metrics) -> Option<LayoutJob> {
    let (transliteration, translation) = word_notes(cell, m);
    let mut format = egui::TextFormat::simple(FontId::proportional(m.meaning_font), m.muted);
    format.line_height = Some(m.meaning_font * 1.25);
    // Sizes are not rounded to whole pixels: a line's width must follow the shared
    // scale exactly, or the scale that makes the widest line fit would never settle.
    let mut job = LayoutJob { halign: Align::Center, round_output_to_gui: false, ..Default::default() };
    job.wrap.max_width = m.meaning_max_width;
    if let Some(text) = &transliteration {
        job.append(text, 0.0, egui::TextFormat { italics: true, ..format.clone() });
    }
    if let Some(text) = &translation {
        if !job.text.is_empty() {
            job.append("\n", 0.0, format.clone());
        }
        job.append(text, 0.0, format);
    }
    (!job.text.is_empty()).then_some(job)
}

/// The meaning under the Bismillah line.
pub fn bismillah_job(text: &str, m: &Metrics) -> LayoutJob {
    job(text, m.meaning_font / 0.72 * 0.8, m.line_width, 1.4, Align::Center, m.muted)
}

/// The note in the box of a verse shown as flowing text.
pub fn note_job(text: &str, m: &Metrics) -> LayoutJob {
    job(text, 12.8, (m.line_width - 16.0).max(40.0), 1.4, Align::LEFT, m.muted)
}

/// Width of a word that has no sprite yet, from the length of its text.
fn placeholder_width(cell: &WordCell, em: f32) -> f32 {
    if cell.is_end {
        return em * 0.75;
    }
    let letters = cell.text.as_deref().map_or(3, |t| t.chars().filter(|c| c.is_alphabetic()).count());
    em * (0.3 + 0.2 * letters as f32).min(2.4)
}

fn place_cell(input: &mut Input<'_>, item: usize, index: usize, cell: &WordCell, settled: &mut bool) -> Word {
    let m = *input.metrics;
    let key = word_key(&cell.location);
    let sprite = input.sprites.and_then(|s| s.get(&key));
    let (mode, glyph_width) = match sprite {
        Some(s) if s.glyph => (Mode::Glyph, s.advance * m.em),
        Some(s) => (Mode::Text { ringed: cell.is_end && !cell.qpc }, s.advance * m.em),
        None if cell.glyph.is_none() && cell.text.as_deref().is_none_or(|t| t.trim().is_empty()) => {
            (Mode::Missing, m.em * 0.9)
        }
        None => (Mode::Placeholder, placeholder_width(cell, m.em)),
    };
    // Final when it is a glyph, or text that no glyph will replace.
    *settled &= match mode {
        Mode::Glyph | Mode::Missing => true,
        Mode::Text { .. } => input.settled || cell.glyph.is_none(),
        Mode::Placeholder => false,
    };
    let glyph_width = if matches!(mode, Mode::Text { ringed: true }) { glyph_width.max(m.em * 0.95) } else { glyph_width };
    let meaning = meaning_job(cell, &m).filter(|_| m.meanings).map(|job| (input.layout)(job).size());
    let width = glyph_width.max(meaning.map_or(0.0, |size| size.x));
    Word { key, x: 0.0, width, y: 0.0, meaning, cell: Some((item, index)), mode }
}

/// Height of the meanings under a row of words, with the gap above them.
fn meanings_height(words: &[Word], m: &Metrics) -> f32 {
    let tallest = words.iter().filter_map(|w| w.meaning).map(|size| size.y).fold(0.0, f32::max);
    if tallest > 0.0 { (m.meaning_gap + tallest).max(0.0) } else { 0.0 }
}

/// Spread `words` (in reading order) over the line, right to left. Returns the
/// width the line needs at its smallest gaps.
fn spread(words: &mut [Word], align: LineAlign, m: &Metrics) -> f32 {
    let n = words.len();
    let content: f32 = words.iter().map(|w| w.width).sum();
    let gaps = n.saturating_sub(1) as f32;
    let min_gap = match align {
        LineAlign::Justify => m.word_gap,
        _ => m.loose_gap,
    };
    let natural = content + gaps * min_gap;
    // A line that doesn't fit (until the shared scale has caught up) is squeezed, never cut.
    let squeeze = if natural > m.line_width && natural > 0.0 { m.line_width / natural } else { 1.0 };
    let gap = match align {
        LineAlign::Justify if n > 1 => ((m.line_width - content * squeeze) / gaps).max(0.0),
        _ => min_gap * squeeze,
    };
    let used = content * squeeze + gaps * gap;
    let mut right = match align {
        LineAlign::Justify | LineAlign::Start => m.line_width,
        LineAlign::Center => (m.line_width + used) / 2.0,
        LineAlign::End => used,
    };
    for word in words.iter_mut() {
        let slot = word.width * squeeze;
        // Centre the word's own box in its (possibly squeezed) slot.
        word.x = right - slot + (slot - word.width) / 2.0;
        right -= slot + gap;
    }
    natural
}

fn bismillah_row(input: &mut Input<'_>, chapter: u16) -> (Vec<Word>, String, bool) {
    let m = *input.metrics;
    let sprites = input.sprites;
    let glyphs: Vec<(u64, &Sprite)> = (0..4)
        .map_while(|i| {
            let key = label_key("b", chapter, i);
            sprites.and_then(|s| s.get(&key)).filter(|s| s.glyph).map(|s| (key, s))
        })
        .collect();
    let text_key = label_key("bt", chapter, 0);
    let mut settled = true;
    let mut words: Vec<Word> = if glyphs.len() == 4 {
        glyphs
            .into_iter()
            .map(|(key, s)| Word { key, x: 0.0, width: s.advance * m.em, y: 0.0, meaning: None, cell: None, mode: Mode::Glyph })
            .collect()
    } else if let Some(s) = sprites.and_then(|s| s.get(&text_key)) {
        settled = input.settled;
        let mode = Mode::Text { ringed: false };
        vec![Word { key: text_key, x: 0.0, width: s.advance * m.em, y: 0.0, meaning: None, cell: None, mode }]
    } else {
        settled = false;
        vec![Word { key: text_key, x: 0.0, width: m.em * 6.0, y: 0.0, meaning: None, cell: None, mode: Mode::Placeholder }]
    };
    // The Bismillah's words sit closer than a line's (`word-spacing: .12em`).
    let tight = Metrics { loose_gap: m.em * 0.12, ..m };
    spread(&mut words, LineAlign::Center, &tight);
    let meaning = input
        .bismillah
        .and_then(|v| v.translations.first())
        .map(|t| strip_html(&t.text))
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| BISMILLAH_MEANING.to_string());
    (words, meaning, settled)
}

/// Translation text without its markup (footnote marks and tags).
pub fn strip_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('<') {
        out.push_str(&rest[..open]);
        let after = &rest[open..];
        let Some(close) = after.find('>') else {
            rest = "";
            break;
        };
        // A footnote mark's number goes with its tag.
        rest = match after[..close].starts_with("<sup") {
            true => after.find("</sup>").map_or("", |end| &after[end + 6..]),
            false => &after[close + 1..],
        };
    }
    out.push_str(rest);
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn flow_note(reason: FlowReason, verse_key: &str) -> String {
    match reason {
        FlowReason::NoLayout => format!("Mushaf line layout unavailable for {verse_key}; shown as flowing text."),
        FlowReason::VerseText => format!("Word data incomplete for {verse_key}; showing the full verse text."),
    }
}

/// Lay a page out. Every cell of the block gets a place.
pub fn place(mut input: Input<'_>, stamp: u64) -> Placed {
    let m = *input.metrics;
    let mut rows = Vec::with_capacity(input.block.items.len());
    let mut y = PAGE_MARGIN_TOP + m.em * PAGE_PAD_TOP;
    let mut natural: f32 = 0.0;
    let mut settled = input.sprites.is_some();
    let mut word_count = 0;
    let block = input.block;
    for (i, item) in block.items.iter().enumerate() {
        let (kind, height) = match item {
            Item::SurahHeader { chapter } => (RowKind::Banner { chapter: *chapter }, m.line_height() + m.line_gap),
            Item::Bismillah { chapter } => {
                let (words, meaning, done) = bismillah_row(&mut input, *chapter);
                settled &= done;
                natural = natural.max(words.iter().map(|w| w.width).sum::<f32>() + m.em * 0.36);
                let meaning_height = (input.layout)(bismillah_job(&meaning, &m)).size().y;
                let height = m.line_height() + m.meaning_gap.max(0.0) + meaning_height + m.line_gap;
                (RowKind::Bismillah { words, meaning }, height)
            }
            Item::Line { cells, align, .. } => {
                let mut words: Vec<Word> =
                    cells.iter().enumerate().map(|(j, c)| place_cell(&mut input, i, j, c, &mut settled)).collect();
                word_count += words.len();
                natural = natural.max(spread(&mut words, *align, &m));
                let height = m.line_height() + meanings_height(&words, &m) + m.line_gap;
                (RowKind::Line { words }, height)
            }
            Item::Flow { verse_key, reason, cells } => {
                let note = flow_note(*reason, verse_key);
                let note_height = (input.layout)(note_job(&note, &m)).size().y;
                let mut words: Vec<Word> =
                    cells.iter().enumerate().map(|(j, c)| place_cell(&mut input, i, j, c, &mut settled)).collect();
                word_count += words.len();
                let height = flow(&mut words, note_height + 8.0, &m) + 8.0 + m.line_gap;
                (RowKind::Flow { note, words }, height)
            }
        };
        rows.push(Row { item: i, y, height, kind });
        y += height;
    }
    let pill_y = y + PILL_MARGIN_TOP;
    let height = pill_y + PILL_HEIGHT + PAGE_PAD_BOTTOM + 1.0 + PAGE_MARGIN_BOTTOM;
    Placed { stamp, rows, height, pill_y, natural: (settled && natural > 0.0).then_some(natural), word_count }
}

/// Wrap the words of a flowing block right to left inside its box; returns the
/// height of the box's content.
fn flow(words: &mut [Word], top: f32, m: &Metrics) -> f32 {
    let (pad, gap) = (8.0, m.em * 0.25);
    let width = m.line_width - 2.0 * pad;
    let (mut right, mut y) = (m.line_width - pad, top);
    let mut start = 0;
    for i in 0..words.len() {
        // An over-wide word still gets a line of its own, shrunk to the box.
        words[i].width = words[i].width.min(width);
        if right - words[i].width < pad && i > start {
            y += m.line_height() + meanings_height(&words[start..i], m);
            right = m.line_width - pad;
            start = i;
        }
        words[i].x = right - words[i].width;
        words[i].y = y;
        right -= words[i].width + gap;
    }
    if words.is_empty() { y } else { y + m.line_height() + meanings_height(&words[start..], m) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metrics(line_width: f32) -> Metrics {
        Metrics {
            fit: 1.0,
            em: 50.0,
            line_width,
            word_gap: 6.0,
            loose_gap: 11.0,
            line_gap: 20.0,
            meaning_gap: 6.0,
            meaning_font: 12.0,
            meaning_max_width: 120.0,
            meanings: false,
            translation: false,
            transliteration: false,
            tooltip: false,
            muted: Color32::GRAY,
        }
    }

    fn words(widths: &[f32]) -> Vec<Word> {
        widths
            .iter()
            .map(|w| Word { key: 0, x: 0.0, width: *w, y: 0.0, meaning: None, cell: None, mode: Mode::Glyph })
            .collect()
    }

    #[test]
    fn justified_line_runs_edge_to_edge_right_to_left() {
        let mut line = words(&[100.0, 50.0, 150.0]);
        let natural = spread(&mut line, LineAlign::Justify, &metrics(600.0));
        assert_eq!(natural, 312.0);
        // First word at the right edge, last at the left edge, equal gaps between.
        assert_eq!((line[0].x + line[0].width, line[2].x), (600.0, 0.0));
        let gap = line[0].x - (line[1].x + line[1].width);
        assert!((gap - 150.0).abs() < 1e-3 && ((line[1].x - (line[2].x + line[2].width)) - gap).abs() < 1e-3);
        // A single word starts at the right.
        let mut one = words(&[80.0]);
        spread(&mut one, LineAlign::Justify, &metrics(600.0));
        assert_eq!(one[0].x, 520.0);
    }

    #[test]
    fn centred_and_partial_lines_are_packed() {
        let m = metrics(600.0);
        let mut line = words(&[100.0, 100.0]);
        assert_eq!(spread(&mut line, LineAlign::Center, &m), 211.0);
        assert!((line[0].x - 305.5).abs() < 1e-3 && (line[1].x - 194.5).abs() < 1e-3);
        spread(&mut line, LineAlign::Start, &m);
        assert_eq!((line[0].x, line[1].x), (500.0, 389.0));
        spread(&mut line, LineAlign::End, &m);
        assert_eq!((line[0].x, line[1].x), (111.0, 0.0));
    }

    #[test]
    fn a_line_too_wide_is_squeezed_into_the_line_never_cut() {
        let m = metrics(300.0);
        let mut line = words(&[200.0, 200.0, 200.0]);
        let natural = spread(&mut line, LineAlign::Justify, &m);
        assert_eq!(natural, 612.0);
        for w in &line {
            // Every word's centre is inside the line, in order from the right.
            let centre = w.x + w.width / 2.0;
            assert!((0.0..=300.0).contains(&centre), "{centre}");
        }
        assert!(line[0].x > line[1].x && line[1].x > line[2].x);
    }

    #[test]
    fn flowing_words_wrap_inside_the_box() {
        let m = metrics(300.0);
        let mut line = words(&[120.0, 120.0, 120.0, 500.0]);
        let height = flow(&mut line, 30.0, &m);
        assert_eq!((line[0].y, line[1].y, line[2].y, line[3].y), (30.0, 30.0, 105.0, 180.0));
        assert!(line.iter().all(|w| w.x >= 8.0 - 1e-3 && w.x + w.width <= 292.0 + 1e-3));
        assert_eq!(height, 255.0);
    }

    #[test]
    fn markup_is_stripped_from_translations() {
        assert_eq!(strip_html("In the Name<sup foot_note=1>1</sup> of <i>Allah</i>"), "In the Name of Allah");
        assert_eq!(strip_html("plain  text"), "plain text");
        assert_eq!(strip_html("broken <tag"), "broken");
    }
}
