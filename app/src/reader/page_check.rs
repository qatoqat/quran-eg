//! The full page-by-page render check: every mushaf page, with real verse data
//! and the real page fonts, taken through what the reader does — layout into
//! lines, the sprite sheet, placement at the fitted scale — and verified.
//!
//! ```sh
//! cargo test -p quran-eg --release page_check -- --ignored --nocapture
//! ```
//!
//! Inputs are read from disk, nothing is fetched:
//! - `QURAN_EG_CHECK_VERSES` (default `../quran-dx/target/reference-cache`): the
//!   API's `/verses/by_chapter` responses with `code_v2`, page and line per word,
//!   as quran-dx's `tools/reference/build.py` leaves them;
//! - `QURAN_EG_CHECK_FONTS` (default `target/tmp/all`): `v2/p{N}.woff2` and
//!   `v4/p{N}.woff2` for all 604 pages.
//!
//! Each page is checked with both fonts, with and without word meanings. A line
//! per page and configuration goes to `target/tmp/page-check.txt`; the test
//! fails if any page has a problem.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::Arc;

use quran_core::api::{Verse, VersesResponse, parse_json};
use quran_core::mushaf_layout::{Item, LineAlign, PageBlock, Trim, build_layout};
use quran_core::parse::{CHAPTER_COUNT, MUSHAF_PAGES};
use quran_core::settings::{MushafFont, Settings};
use quran_core::{integrity, reference};

use super::blocks::{self, cell_count};
use super::place::{self, Metrics, Mode, Placed, RowKind, label_key, word_key};
use super::{COLUMN_PAD, metrics_for};
use crate::glyphs::{PageGlyphs, Palette, parse_page_font};
use crate::sheet::{self, Sheet, Sprite};

const EPS: f32 = 0.05;

fn workspace() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn dir(var: &str, default: &str) -> PathBuf {
    std::env::var_os(var).map_or_else(|| workspace().join(default), PathBuf::from)
}

/// Every verse of the Quran with its words, in order.
fn load_verses() -> Vec<Verse> {
    let dir = dir("QURAN_EG_CHECK_VERSES", "../quran-dx/target/reference-cache");
    let mut verses = Vec::with_capacity(6236);
    for chapter in 1..=CHAPTER_COUNT {
        for page in 1.. {
            let name = format!(
                "verses_by_chapter_{chapter}_words=true_per_page=50_page={page}_mushaf=19_word_fields=code_v2,v2_page,page_number,line_number.json"
            );
            let Ok(body) = std::fs::read_to_string(dir.join(&name)) else { break };
            let response: VersesResponse = parse_json(&body).unwrap_or_else(|e| panic!("{name}: {e}"));
            verses.extend(response.verses);
        }
    }
    assert_eq!(verses.len(), 6236, "verse data incomplete in {}", dir.display());
    verses
}

fn load_font(set: MushafFont, page: u16) -> Arc<dyn PageGlyphs> {
    let folder = match set {
        MushafFont::MadaniV2 => "v2",
        MushafFont::TajweedV4 => "v4",
    };
    let path = dir("QURAN_EG_CHECK_FONTS", "target/tmp/all").join(folder).join(format!("p{page}.woff2"));
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    parse_page_font(&bytes).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// Whether a sprite part's rectangle of the sheet has any ink.
fn has_ink(sheet: &Sheet, part: &sheet::Part) -> bool {
    let x = |u: f32| (u * sheet.width as f32).round() as usize;
    let y = |v: f32| (v * sheet.height as f32).round() as usize;
    (y(part.uv[1])..y(part.uv[3])).any(|row| (x(part.uv[0])..x(part.uv[2])).any(|col| sheet.rgba[(row * sheet.width + col) * 4 + 3] != 0))
}

/// A drawable glyph: every part drawn, ink somewhere, and a part without ink takes no room.
fn glyph_problem(sheet: &Sheet, sprite: Option<&Sprite>) -> Option<String> {
    let Some(sprite) = sprite else { return Some("has no glyph in the page font".into()) };
    if !sprite.glyph {
        return Some("is drawn as text, not as its glyph".into());
    }
    let inked: Vec<bool> = sprite.parts.iter().map(|p| has_ink(sheet, p)).collect();
    if !inked.contains(&true) {
        return Some("draws nothing".into());
    }
    let idle = sprite.parts.iter().zip(&inked).any(|(p, ink)| !ink && p.advance != 0.0);
    idle.then(|| "has an empty glyph that takes room".into())
}

struct PageResult {
    problems: Vec<String>,
    lines: usize,
    words: usize,
    fit: f32,
    line_width: f32,
}

/// What a page needs besides its own words and font.
struct Shared<'a> {
    first_verse: &'a Verse,
    first_font: Arc<dyn PageGlyphs>,
    settings: &'a Settings,
    ctx: &'a egui::Context,
}

fn build_sheet(block: &PageBlock, font: &Arc<dyn PageGlyphs>, shared: &Shared<'_>, px_per_em: f32) -> Sheet {
    let job = sheet::Job {
        requests: blocks::requests(block, &[], Some(shared.first_verse)),
        fonts: vec![Some(font.clone()), Some(shared.first_font.clone())],
        // No fallback text: a word the font can't draw must show up as a problem.
        fallback: None,
        ornaments: None,
        px_per_em,
        palette: Palette::Light,
    };
    sheet::build(&job)
}

fn place_page(block: &PageBlock, sheet: &Sheet, m: &Metrics, shared: &Shared<'_>) -> Placed {
    let mut placed = None;
    let mut output = shared.ctx.run_ui(egui::RawInput::default(), |ui| {
        placed = Some(ui.ctx().fonts_mut(|fonts| {
            let input = place::Input {
                block,
                metrics: m,
                sprites: Some(&sheet.sprites),
                settled: true,
                bismillah: Some(shared.first_verse),
                layout: &mut |job| fonts.layout_job(job),
            };
            place::place(input, 0)
        }));
    });
    output.textures_delta.clear();
    placed.expect("the frame ran")
}

fn check_page(page: u16, verses: &[Verse], font: &Arc<dyn PageGlyphs>, shared: &Shared<'_>) -> PageResult {
    let mut problems: Vec<String> = Vec::new();
    let column = quran_core::settings::content_width_px(shared.settings.content_width) as f32 - 2.0 * COLUMN_PAD;
    let muted = egui::Color32::GRAY;

    // The page's verses, grouped into its lines as the page view does.
    let on_page = reference::verses_on_page(page);
    let mine: Vec<Verse> = verses
        .iter()
        .filter(|v| v.chapter_number().is_some_and(|c| on_page.iter().any(|(ch, span)| *ch == c && span.contains(v.verse_number))))
        .cloned()
        .collect();
    problems.extend(integrity::check_all(&mine, "text", &on_page, reference::verses_in).iter().map(ToString::to_string));
    problems.extend(integrity::line_issues(&mine, Some(page)).iter().map(ToString::to_string));
    let layout = build_layout(&mine, Trim { page: Some(page), ..Trim::default() });
    let [block] = layout.pages.as_slice() else {
        problems.push(format!("laid out as {} pages instead of one", layout.pages.len()));
        return PageResult { problems, lines: 0, words: 0, fit: 1.0, line_width: 0.0 };
    };
    if block.page != Some(page) {
        problems.push(format!("laid out as page {:?}", block.page));
    }

    // Lines: exactly the printed ones, each once, with the printed number of words.
    let mut found = [0usize; 16];
    let mut seen_lines = HashSet::new();
    for item in &block.items {
        match item {
            Item::Line { line, cells, .. } => {
                if !seen_lines.insert(*line) {
                    problems.push(format!("line {line} is split in two"));
                }
                found[usize::from(*line).min(15)] += cells.len();
            }
            Item::Flow { verse_key, .. } => problems.push(format!("{verse_key} is not on its lines (shown as flowing text)")),
            _ => {}
        }
    }
    for line in 1..=15u16 {
        let printed = reference::words_on_line(page, line).unwrap_or(0);
        if found[usize::from(line)] != printed {
            problems.push(format!("line {line}: {} of {printed} words", found[usize::from(line)]));
        }
    }

    // Banner and Bismillah exactly where a surah starts: right before the line of its first word.
    let starts_surah = |i: usize| match block.items.get(i) {
        Some(Item::Line { cells, .. }) => cells.first().and_then(|c| c.location.strip_suffix(":1:1")).and_then(|c| c.parse::<u16>().ok()),
        _ => None,
    };
    for (i, item) in block.items.iter().enumerate() {
        match item {
            Item::SurahHeader { chapter } => {
                let bismillah = !matches!(*chapter, 1 | 9);
                let after = if bismillah { i + 2 } else { i + 1 };
                if bismillah && !matches!(block.items.get(i + 1), Some(Item::Bismillah { chapter: c }) if c == chapter) {
                    problems.push(format!("surah {chapter}: no Bismillah line after its banner"));
                }
                if starts_surah(after) != Some(*chapter) {
                    problems.push(format!("surah {chapter}: its banner is not right before its first line"));
                }
            }
            Item::Bismillah { chapter } => {
                if !matches!(i.checked_sub(1).and_then(|j| block.items.get(j)), Some(Item::SurahHeader { chapter: c }) if c == chapter) {
                    problems.push(format!("surah {chapter}: a Bismillah line without its banner"));
                }
            }
            Item::Line { .. } => {
                if let Some(chapter) = starts_surah(i) {
                    let before = if matches!(chapter, 1 | 9) { 1 } else { 2 };
                    let banner = i.checked_sub(before).and_then(|j| block.items.get(j));
                    if !matches!(banner, Some(Item::SurahHeader { chapter: c }) if *c == chapter) {
                        problems.push(format!("surah {chapter} starts without its banner"));
                    }
                }
            }
            Item::Flow { .. } => {}
        }
    }

    // The widest line decides the scale, as in the reader: place at scale 1, then at the fitted scale.
    let base = metrics_for(shared.settings, muted, column, None);
    let first_sheet = build_sheet(block, font, shared, base.em);
    let mut widest = place_page(block, &first_sheet, &base, shared).natural;
    let mut m = metrics_for(shared.settings, muted, column, widest);
    let mut placed = place_page(block, &first_sheet, &m, shared);
    for _ in 0..4 {
        // (The reader keeps the widest measurement and goes again until it holds.)
        let again = placed.natural.map(|n| n / m.fit).into_iter().chain(widest).reduce(f32::max);
        if again == widest {
            break;
        }
        widest = again;
        m = metrics_for(shared.settings, muted, column, widest);
        placed = place_page(block, &first_sheet, &m, shared);
    }
    // The glyphs as they are drawn at that scale.
    let sheet = if (m.em - base.em).abs() > 0.01 { build_sheet(block, font, shared, m.em) } else { first_sheet };

    // Glyphs: every word and marker has one that draws, in the page font.
    let mut keys = HashSet::new();
    let mut order: Vec<(u16, u16, u16)> = Vec::new();
    for item in &block.items {
        let Item::Line { cells, .. } = item else { continue };
        for cell in cells {
            let key = word_key(&cell.location);
            if !keys.insert(key) {
                problems.push(format!("{} is on the page twice", cell.location));
            }
            if let Some(problem) = glyph_problem(&sheet, sheet.sprites.get(&key)) {
                problems.push(format!("{} {problem} ({:?})", cell.location, cell.glyph));
            }
            let mut parts = cell.location.split(':').filter_map(|n| n.parse::<u16>().ok());
            order.push((parts.next().unwrap_or(0), parts.next().unwrap_or(0), parts.next().unwrap_or(0)));
        }
    }
    if !order.is_sorted() {
        problems.push("its words are not in reading order".into());
    }
    if !sheet.failed_glyphs.is_empty() {
        problems.push(format!("{} glyphs failed to draw", sheet.failed_glyphs.len()));
    }

    // Placement: every word once, in its line, right to left, inside the line; no line too wide.
    if placed.word_count != cell_count(block) || placed.rows.len() != block.items.len() {
        problems.push(format!("{} of {} words placed in {} of {} rows", placed.word_count, cell_count(block), placed.rows.len(), block.items.len()));
    }
    if placed.natural.is_none() {
        problems.push("the page's width is not settled (a word is not in its final form)".into());
    }
    if m.line_width > column + EPS {
        problems.push(format!("lines are {:.1} wide in a column of {column:.1}", m.line_width));
    }
    let mut bottom = 0.0f32;
    for (i, row) in placed.rows.iter().enumerate() {
        if row.item != i || row.y + EPS < bottom || row.height <= 0.0 {
            problems.push(format!("row {i} is out of place"));
        }
        bottom = row.y + row.height;
        let words = row.words();
        match (&row.kind, &block.items[i]) {
            (RowKind::Line { .. }, Item::Line { line, cells, align }) => {
                if words.len() != cells.len() {
                    problems.push(format!("line {line}: {} of {} words placed", words.len(), cells.len()));
                    continue;
                }
                let min_gap = if *align == LineAlign::Justify { m.word_gap } else { m.loose_gap };
                let needs: f32 = words.iter().map(|w| w.width).sum::<f32>() + (words.len() - 1) as f32 * min_gap;
                if needs > m.line_width + EPS {
                    problems.push(format!("line {line} needs {needs:.1} of {:.1}: it overflows", m.line_width));
                }
                for (j, word) in words.iter().enumerate() {
                    let at = &cells[j].location;
                    if word.cell != Some((i, j)) || word.key != word_key(at) {
                        problems.push(format!("{at} is not at its place in line {line}"));
                    }
                    if word.mode != Mode::Glyph {
                        problems.push(format!("{at} is placed as {:?}", word.mode));
                    }
                    if word.y != 0.0 {
                        problems.push(format!("{at} wrapped out of line {line}"));
                    }
                    if word.x < -EPS || word.x + word.width > m.line_width + EPS {
                        problems.push(format!("{at} reaches outside line {line}"));
                    }
                    if j > 0 && word.x + word.width > words[j - 1].x - min_gap + EPS {
                        problems.push(format!("{at} is closer than the smallest gap to the word before it"));
                    }
                }
                if *align == LineAlign::Justify && words.len() > 1 {
                    let (first, last) = (&words[0], &words[words.len() - 1]);
                    if (first.x + first.width - m.line_width).abs() > EPS || last.x.abs() > EPS {
                        problems.push(format!("line {line} is not justified edge to edge"));
                    }
                }
            }
            (RowKind::Bismillah { words, .. }, Item::Bismillah { chapter }) => {
                let glyphs = (0..4).filter(|part| glyph_problem(&sheet, sheet.sprites.get(&label_key("b", *chapter, *part))).is_none()).count();
                if glyphs != 4 || words.len() != 4 || words.iter().any(|w| w.mode != Mode::Glyph) {
                    problems.push(format!("surah {chapter}: the Bismillah is not drawn from its four glyphs"));
                }
            }
            (RowKind::Banner { .. }, Item::SurahHeader { .. }) => {}
            _ => problems.push(format!("row {i} is not what the page has there")),
        }
    }
    PageResult { problems, lines: seen_lines.len(), words: cell_count(block), fit: m.fit, line_width: m.line_width }
}

#[test]
#[ignore = "reads all 604 pages' verses and fonts from disk (see the module's notes)"]
fn every_page_renders_whole() {
    let verses = load_verses();
    let first_verse = verses.iter().find(|v| v.verse_key == "1:1").expect("1:1");
    let ctx = egui::Context::default();
    let mut report = String::new();
    let mut failed = 0usize;
    let mut total_words = HashMap::new();
    for (font, meanings) in [(MushafFont::TajweedV4, true), (MushafFont::TajweedV4, false), (MushafFont::MadaniV2, true), (MushafFont::MadaniV2, false)] {
        let mut settings = Settings { mushaf_font: font, ..Settings::default() };
        settings.word_by_word.show_translation = meanings;
        let shared = Shared { first_verse, first_font: load_font(font, 1), settings: &settings, ctx: &ctx };
        let name = format!("{font:?}, meanings {}", if meanings { "on" } else { "off" });
        let (mut words, mut lines, mut scaled, mut bad) = (0usize, 0usize, 0usize, 0usize);
        let (mut smallest, mut widest_line) = (1.0f32, 0.0f32);
        for page in 1..=MUSHAF_PAGES {
            let result = check_page(page, &verses, &load_font(font, page), &shared);
            words += result.words;
            lines += result.lines;
            scaled += usize::from(result.fit < 1.0);
            smallest = smallest.min(result.fit);
            widest_line = widest_line.max(result.line_width);
            let verdict = if result.problems.is_empty() { "ok".to_string() } else { format!("{} PROBLEMS", result.problems.len()) };
            let _ = writeln!(
                report,
                "{name}: page {page}: {verdict}; {} lines, {} words, scale {:.3}, line width {:.1}",
                result.lines, result.words, result.fit, result.line_width
            );
            if !result.problems.is_empty() {
                bad += 1;
                for problem in result.problems.iter().take(12) {
                    let _ = writeln!(report, "    {problem}");
                    if bad <= 5 {
                        println!("{name}: page {page}: {problem}");
                    }
                }
            }
        }
        println!(
            "{name}: {} pages, {bad} with problems; {lines} lines, {words} words and markers; {scaled} pages scaled down (smallest scale {smallest:.3}); widest line {widest_line:.1}"
        , MUSHAF_PAGES);
        failed += bad;
        total_words.insert(name, words);
    }
    let path = workspace().join("target/tmp/page-check.txt");
    let _ = std::fs::create_dir_all(path.parent().expect("a folder"));
    std::fs::write(&path, &report).expect("the report can be written");
    println!("report: {}", path.display());
    // Every word and every marker of the Quran was on exactly one page, in every configuration.
    let expected: usize = verses.iter().map(|v| v.words.len()).sum();
    assert!(total_words.values().all(|n| *n == expected), "{total_words:?} of {expected} words");
    assert_eq!(failed, 0, "{failed} page checks found problems; see {}", path.display());
}
