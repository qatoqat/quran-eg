//! A reader view run without a window or a network: verses made up on real
//! verse keys go in, frames are run, and what the view holds is checked.

use egui::{Pos2, RawInput, Rect, vec2};
use quran_core::api::{Client, Pagination, Verse, VersesResponse};
use quran_core::feed::{Feed, VerseSource};
use quran_core::mushaf_layout::Item;
use quran_core::parse::VerseSpan;
use quran_core::settings::Settings;
use quran_core::view::{View, WordRef};
use serde_json::json;

use super::blocks::cell_count;
use super::place::Mode;
use super::{Env, Reader};
use crate::fonts::FontStore;
use crate::theme::Colors;

/// A verse on one line, with as many words as it really has (the layout
/// checks word counts against the reference tables).
pub fn verse(key: &str, page: u16, line: u16) -> Verse {
    let (c, n) = key.split_once(':').unwrap();
    let (c, n): (u16, u16) = (c.parse().unwrap(), n.parse().unwrap());
    let count = quran_core::reference::words_in(c, n).unwrap() as u16;
    let word = |pos: u16, kind: &str| {
        json!({ "id": pos, "position": pos, "char_type_name": kind, "text_uthmani": "نص",
                "code_v2": char::from_u32(0xfc40 + u32::from(pos)).unwrap().to_string(),
                "translation": { "text": "a meaning" },
                "page_number": page, "line_number": line, "v2_page": page })
    };
    let mut words: Vec<_> = (1..=count).map(|pos| word(pos, "word")).collect();
    words.push(word(count + 1, "end"));
    serde_json::from_value(json!({
        "id": 1, "verse_number": n, "verse_key": key, "text_uthmani": "نص", "words": words
    }))
    .unwrap()
}

/// Verses 2:1-2:60, one per line, fifteen lines to a page: pages 3-6 (every
/// line of pages 1 and 2 is centred, as printed).
fn sixty_verses() -> Vec<Verse> {
    (0..60u16).map(|i| verse(&format!("2:{}", i + 1), 3 + i / 15, 1 + i % 15)).collect()
}

struct Bench {
    ctx: egui::Context,
    settings: Settings,
    colors: Colors,
    fonts: FontStore,
    client: Client,
    reader: Reader,
}

impl Bench {
    /// A surah view holding `verses`, with no font source at all.
    fn new(verses: Vec<Verse>) -> Self {
        let ctx = egui::Context::default();
        let settings = Settings::default();
        let colors = Colors::of(&settings, false);
        let mut fonts = FontStore::new(&ctx, Vec::new(), None);
        // Nothing listens here: a request that slipped through would fail at once.
        let client = Client::new("http://127.0.0.1:9");
        let total = verses.len() as u32;
        let response = VersesResponse {
            verses,
            pagination: Pagination { per_page: 50, current_page: 1, next_page: None, total_pages: 1, total_records: total },
        };
        let env = Env { settings: &settings, colors: &colors, fonts: &mut fonts, chapters: &[], juzs: &[], client: &client, bismillah: None, lens: None };
        // The range the verses cover, so the view is complete.
        let span = Some(VerseSpan { from: 1, to: total as u16 });
        let view = View::Surah { chapter: 2, span };
        let mut reader = Reader::closed(view, Feed::new(VerseSource::Chapter { chapter: 2, span }), &env);
        reader.feed.merge(1, response);
        reader.rebuild(&env);
        Self { ctx, settings, colors, fonts, client, reader }
    }

    /// Run `count` frames of a 1000 × 800 window.
    fn frames(&mut self, count: usize) {
        for _ in 0..count {
            let input = RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 800.0))), ..Default::default() };
            let mut output = self.ctx.run_ui(input, |ui| {
                self.fonts.poll();
                let mut env = Env {
                    settings: &self.settings,
                    colors: &self.colors,
                    fonts: &mut self.fonts,
                    chapters: &[],
                    juzs: &[],
                    client: &self.client,
                    bismillah: None,
                    lens: None,
                };
                self.reader.ui(ui, &mut env);
            });
            // No renderer takes the textures here.
            output.textures_delta.clear();
        }
    }

    fn placed(&self) -> Vec<usize> {
        self.reader.blocks.iter().enumerate().filter(|(_, b)| b.placed.is_some()).map(|(i, _)| i).collect()
    }
}

#[test]
fn every_word_gets_a_place_on_its_line_even_without_any_font() {
    let mut bench = Bench::new(sixty_verses());
    bench.frames(3);
    assert_eq!(bench.reader.blocks.len(), 4);
    assert_eq!(bench.placed(), [0], "only the page at the viewport is laid out");

    let block = &bench.reader.blocks[0];
    let placed = block.placed.as_ref().unwrap();
    assert_eq!(placed.word_count, cell_count(&block.page));
    assert_eq!(placed.word_count, block.cells);
    // Banner, Bismillah, then the fifteen lines, top to bottom without overlap.
    assert_eq!(placed.rows.len(), block.page.items.len());
    assert_eq!(placed.rows.len(), 17);
    for pair in placed.rows.windows(2) {
        assert!(pair[0].height > 0.0 && (pair[0].y + pair[0].height - pair[1].y).abs() < 0.01);
    }
    let metrics = bench.reader.metrics_now(&bench.settings, &bench.colors);
    let line_width = metrics.line_width;
    let (mut fitting, mut squeezed) = (0, 0);
    for row in placed.rows.iter().filter(|r| r.is_text()) {
        let Item::Line { cells, .. } = &block.page.items[row.item] else { panic!("a line") };
        let words = row.words();
        assert_eq!(words.len(), cells.len(), "line {}", row.item);
        let needs: f32 = words.iter().map(|w| w.width).sum::<f32>() + (words.len() - 1) as f32 * metrics.word_gap;
        for (i, word) in words.iter().enumerate() {
            // No font can come: a marked box stands for the word, with its meaning under it.
            assert_eq!(word.mode, Mode::Placeholder);
            assert!(word.meaning.is_some() && word.width > 0.0);
            assert_eq!(word.cell, Some((row.item, i)));
            // In reading order from the right, each word's middle inside the line.
            let middle = word.x + word.width / 2.0;
            assert!((0.0..=line_width).contains(&middle), "line {}: {middle}", row.item);
            assert!(i == 0 || middle < words[i - 1].x + words[i - 1].width / 2.0);
        }
        if needs <= line_width {
            // A full line that fits runs edge to edge, its words apart.
            fitting += 1;
            assert!((words[0].x + words[0].width - line_width).abs() < 0.01);
            assert!(words.last().unwrap().x.abs() < 0.01);
            assert!(words.windows(2).all(|w| w[1].x + w[1].width <= w[0].x - metrics.word_gap + 0.01));
        } else {
            // (A whole long verse on one line, as only this test has: squeezed in, not cut.)
            squeezed += 1;
        }
    }
    assert!(fitting > 0 && fitting + squeezed == 15, "{fitting} + {squeezed}");
    // Nothing was measured with real glyphs, so nothing claims to know the widest line.
    assert!(placed.natural.is_none() && bench.reader.widest().is_none());
    assert_eq!(bench.fonts.loaded_pages(), 0);
    assert!(bench.fonts.page_failed(quran_core::fonts::FontSet::V4Colr, 3));
    // The reading position is the first word, at the top of the view.
    let position = bench.reader.position.unwrap();
    assert_eq!((position.at, position.page, position.top), (WordRef { chapter: 2, verse: 1, word: 1 }, 3, true));
    assert_eq!(bench.reader.location().to_string(), "/2/1-60");
    // No verse or word is reported; only that these made-up lines are not the printed ones.
    assert!(bench.reader.notice.0.iter().all(|line| line.starts_with("Some mushaf lines")), "{:?}", bench.reader.notice);
}

#[test]
fn pages_far_from_the_viewport_hold_nothing_and_the_position_follows() {
    let mut bench = Bench::new(sixty_verses());
    bench.frames(2);
    let total: f32 = bench.reader.blocks.iter().map(|b| b.height).sum();
    // To the last page.
    bench.reader.scroll = total - 900.0;
    bench.frames(3);
    assert_eq!(bench.placed(), [3], "the first pages were released, the last one laid out");
    let position = bench.reader.position.unwrap();
    assert_eq!((position.at.chapter, position.page, position.top), (2, 6, false));
    assert!(position.at.verse > 45, "{:?}", position.at);
    let at = position.at;
    assert_eq!(bench.reader.location().to_string(), format!("/2/1-60#{at}"));

    // The line at the top stays the line at the top when the window's text size changes.
    bench.settings.arabic_font_scale = 7;
    bench.frames(4);
    assert_eq!(bench.reader.position.unwrap().at, at);
    bench.settings.word_by_word.show_translation = false;
    bench.frames(4);
    assert_eq!(bench.reader.position.unwrap().at, at);
}

#[test]
fn opening_at_a_position_puts_its_line_at_the_top() {
    let mut bench = Bench::new(sixty_verses());
    let target = WordRef { chapter: 2, verse: 40, word: 3 };
    bench.reader.pending = Some(target);
    assert_eq!(bench.reader.location().to_string(), "/2/1-60#2:40:3", "known before it is on screen");
    bench.frames(3);
    // Verse 40 has a line to itself here, so its first word starts the top line.
    let position = bench.reader.position.unwrap();
    assert_eq!((position.at, position.top), (WordRef { chapter: 2, verse: 40, word: 1 }, false));
    assert!(bench.placed().contains(&2));

    // A position that is not in the view is dropped: the view stays at its top.
    let mut bench = Bench::new(sixty_verses());
    bench.reader.pending = Some(WordRef { chapter: 3, verse: 1, word: 1 });
    bench.frames(2);
    assert!(bench.reader.pending.is_none() && bench.reader.position.unwrap().top);
}

#[test]
fn a_bismillah_line_is_four_places_for_the_voice_pointer_and_its_surah_head_can_be_gone_to() {
    let mut bench = Bench::new(sixty_verses());
    bench.frames(3);
    let words = &bench.reader.words;
    let first = words.iter().position(|w| w.0 == (2, 1, 1)).unwrap();
    let line: Vec<_> = words[..first].iter().map(|w| w.0).collect();
    assert_eq!(line, [(2, 0, 1), (2, 0, 2), (2, 0, 3), (2, 0, 4)], "just before the surah's first word");
    // Its one placeholder in quarters, right to left, above the first line.
    let b = &words[..first];
    assert!(b.windows(2).all(|p| (p[1].1.right() - p[0].1.left()).abs() < 0.01) && b[0].1.bottom() <= words[first].1.top());
    // From further on, the surah's head comes into view with its first word.
    bench.reader.scroll = 2500.0;
    bench.frames(2);
    bench.reader.go_to_head(2);
    bench.frames(3);
    let line = bench.reader.words.iter().find(|w| w.0 == (2, 0, 1)).expect("the Bismillah line is painted");
    assert!(line.1.top() > 0.0, "{:?} in view", line.1);
    // A word gone to is at the top: the line above it is not in view.
    bench.reader.go_to(Some(WordRef { chapter: 2, verse: 1, word: 1 }));
    bench.frames(3);
    assert!(bench.reader.words.iter().find(|w| w.0 == (2, 0, 1)).is_none_or(|w| w.1.top() < 0.0));
}
