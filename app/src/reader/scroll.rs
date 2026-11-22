//! Scrolling a reader view: only the pages near the viewport are laid out and
//! hold a sprite sheet, and the line at the top of the viewport stays where it
//! is while the pages around it are measured, rescaled or redrawn.

use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use egui::{Key, Rect, Ui};
use quran_core::fonts::FontSet;
use quran_core::mushaf_layout::{Item, WordCell};
use quran_core::view::{Position, WordRef};

use super::blocks::{self, Block, JobInput, Spec};
use super::place::{self, Metrics};
use super::{Env, MAX_PX_PER_EM, RESIZE_SETTLE, Reader};
use crate::glyphs::Palette;
use crate::task;

/// Where the top of the viewport is inside a page.
enum Anchor {
    /// This far (0..=1) down a row.
    Row { row: usize, part: f32 },
    /// This far (0..=1) down the page, when it has not been laid out.
    Page(f32),
}

fn anchor_in(block: &Block, offset: f32) -> Anchor {
    let row = block.placed.as_ref().and_then(|p| {
        let (i, row) = p.rows.iter().enumerate().rev().find(|(_, r)| r.y <= offset)?;
        (offset < row.y + row.height).then(|| Anchor::Row { row: i, part: (offset - row.y) / row.height.max(1.0) })
    });
    row.unwrap_or(Anchor::Page(offset / block.height.max(1.0)))
}

fn resolve(block: &Block, anchor: &Anchor) -> f32 {
    match anchor {
        Anchor::Row { row, part } => match block.placed.as_ref().and_then(|p| p.rows.get(*row)) {
            Some(r) => r.y + part * r.height,
            None => 0.0,
        },
        Anchor::Page(part) => part * block.height,
    }
}

/// Is the page's font settled: loaded into the sheet, or failed for good?
fn settled(block: &Block, env: &Env<'_>, set: FontSet) -> bool {
    match block.page.page {
        Some(page) => block.sheet.as_ref().is_some_and(|s| s.spec.fonts[0]) || env.fonts.page_failed(set, page),
        None => block.sheet.is_some(),
    }
}

/// A guess at the height of the translations under a page that hasn't drawn them yet.
pub(super) fn below_estimate(block: &Block, env: &Env<'_>) -> f32 {
    let per_verse = 60.0 + 80.0 * env.settings.translations.len().max(1) as f32;
    block.page.verse_keys.len() as f32 * per_verse
}

/// Lay `block` out if what its geometry depends on has changed.
fn place_block(block: &mut Block, ctx: &egui::Context, env: &Env<'_>, m: &Metrics, set: FontSet) {
    let settled = settled(block, env, set);
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    m.hash(&mut hasher);
    (block.sheet.as_ref().map(|s| s.version), block.cells, settled, env.bismillah.is_some()).hash(&mut hasher);
    let stamp = hasher.finish();
    // Verse translations under the page: as measured when drawn, or a guess until then.
    block.below = match (env.settings.page_translations(), block.below > 0.0) {
        (false, _) => 0.0,
        (true, true) => block.below,
        (true, false) => below_estimate(block, env),
    };
    if let Some(placed) = block.placed.as_ref().filter(|p| p.stamp == stamp) {
        block.height = placed.height + block.below;
        return;
    }
    let placed = ctx.fonts_mut(|fonts| {
        let input = place::Input {
            block: &block.page,
            metrics: m,
            sprites: block.sheet.as_ref().map(|s| &s.sprites),
            settled,
            bismillah: env.bismillah,
            layout: &mut |job| fonts.layout_job(job),
        };
        place::place(input, stamp)
    });
    if placed.word_count != block.cells {
        // Cannot happen (every cell of every line is placed), so say so if it does.
        eprintln!("quran-eg: page {:?}: {} of {} words were placed", block.key.0, placed.word_count, block.cells);
    }
    block.height = placed.height + block.below;
    if let Some(natural) = placed.natural {
        // Never narrower than measured before: the shared scale only settles downwards.
        let natural = natural / m.fit;
        block.natural = Some(block.natural.map_or(natural, |old| old.max(natural)));
    }
    block.placed = Some(placed);
}

impl Reader {
    /// Wheel and keys.
    pub(super) fn scroll_input(&mut self, ui: &Ui, area: Rect) {
        let (view, before) = (area.height(), self.scroll);
        if ui.rect_contains_pointer(area) {
            self.scroll -= ui.input(|i| i.smooth_scroll_delta.y);
        }
        self.touched = self.scroll != before;
        if ui.ctx().egui_wants_keyboard_input() {
            return;
        }
        let step = ui.input(|i| {
            let page = view * 0.9;
            let mut step = 0.0;
            for (key, amount) in [
                (Key::ArrowDown, 60.0),
                (Key::ArrowUp, -60.0),
                (Key::PageDown, page),
                (Key::PageUp, -page),
                (Key::Space, if i.modifiers.shift { -page } else { page }),
                (Key::Home, f32::NEG_INFINITY),
                (Key::End, f32::INFINITY),
            ] {
                if i.key_pressed(key) {
                    step += amount;
                }
            }
            step
        });
        self.scroll = (self.scroll + step).max(0.0);
        self.touched = self.scroll != before;
        // (`End` is brought back to the last page by the clamp that follows placement.)
        if self.scroll.is_infinite() {
            self.scroll = self.top_height + self.blocks.iter().map(|b| b.height).sum::<f32>() + self.bottom_height;
        }
    }

    /// Scroll to the position the view was opened at once its verse is there,
    /// loading more verses until it is.
    pub(super) fn open_pending(&mut self, ctx: &egui::Context, env: &Env<'_>, m: &Metrics) {
        let Some(target) = self.pending else { return };
        if !self.feed.has_verse(&target.verse_key()) {
            if self.feed.complete() || self.error.is_some() {
                self.pending = None; // not in this view (or not reachable now): stay at the top
            } else {
                self.load_more(ctx, env.client);
            }
            return;
        }
        self.pending = None;
        let location = target.location();
        // The word itself, else the first word of its verse.
        let find = |matches: &dyn Fn(&WordCell) -> bool| {
            self.blocks.iter().enumerate().find_map(|(b, block)| {
                let holds = |item: &Item| match item {
                    Item::Line { cells, .. } | Item::Flow { cells, .. } => cells.iter().any(matches),
                    _ => false,
                };
                Some((b, block.page.items.iter().position(holds)?))
            })
        };
        let verse_key = target.verse_key();
        let Some((index, mut item)) = find(&|c| c.location == location).or_else(|| find(&|c| c.verse_key == verse_key)) else {
            return;
        };
        if std::mem::take(&mut self.head) {
            let head = |i: &Item| matches!(i, Item::SurahHeader { chapter } | Item::Bismillah { chapter } if *chapter == target.chapter);
            item = self.blocks[index].page.items[..item].iter().position(head).unwrap_or(item);
        }
        let set = FontSet::of(env.settings.mushaf_font);
        place_block(&mut self.blocks[index], ctx, env, m, set);
        let above: f32 = self.blocks[..index].iter().map(|b| b.height).sum();
        let row = self.blocks[index].placed.as_ref().and_then(|p| p.rows.iter().find(|r| r.item == item)).map_or(0.0, |r| r.y);
        // A little above the line: marks on tall letters rise above its box.
        self.scroll = (self.top_height + above + row - m.em * 0.3).max(0.0);
    }

    /// Lay out the pages near the viewport and see to their sheets, keeping the
    /// line at the top of the viewport where it is.
    pub(super) fn place_near(&mut self, ctx: &egui::Context, env: &mut Env<'_>, m: &Metrics, view: f32) {
        let view_top = self.scroll - self.top_height;
        let mut anchor = None;
        let mut top = 0.0;
        for (i, block) in self.blocks.iter().enumerate() {
            if view_top >= top && view_top < top + block.height {
                anchor = Some((i, anchor_in(block, view_top - top)));
                break;
            }
            top += block.height;
        }

        let set = FontSet::of(env.settings.mushaf_font);
        let now = ctx.input(|i| i.time);
        let px_per_em = ((m.em * ctx.pixels_per_point()).min(MAX_PX_PER_EM) * 4.0).round() / 4.0;
        if px_per_em != self.px_per_em {
            self.px_per_em = px_per_em;
            self.px_changed = now;
        }
        let mut top = 0.0;
        for i in 0..self.blocks.len() {
            let height = self.blocks[i].height;
            let near = top + height >= view_top - view && top <= view_top + 2.0 * view;
            top += height;
            if near {
                self.request_sheet(i, ctx, env, set, now);
                place_block(&mut self.blocks[i], ctx, env, m, set);
            }
        }

        if let Some((i, anchor)) = anchor {
            let new_top: f32 = self.blocks[..i].iter().map(|b| b.height).sum();
            self.scroll += new_top + resolve(&self.blocks[i], &anchor) - view_top;
        }
    }

    /// Start drawing block `i`'s sheet if the one it has isn't what is wanted now.
    fn request_sheet(&mut self, i: usize, ctx: &egui::Context, env: &mut Env<'_>, set: FontSet, now: f64) {
        let block = &mut self.blocks[i];
        let page_font = block.page.page.and_then(|p| env.fonts.page(set, p));
        let first_font = if block.has_bismillah() { env.fonts.page(set, 1) } else { None };
        let fallback = env.fonts.fallback();
        let has_banner = block.page.items.iter().any(|i| matches!(i, Item::SurahHeader { .. }));
        let ornaments = if has_banner { env.fonts.ornaments() } else { None };
        let spec = Spec {
            px_per_em: self.px_per_em,
            // A single-colour font has no palettes: the theme tints it when drawn.
            palette: if set.coloured() { env.colors.glyphs } else { Palette::Light },
            set,
            fonts: [page_font.is_some(), first_font.is_some(), fallback.is_some(), ornaments.is_some()],
            content: blocks::content_stamp(block, env.chapters, env.bismillah),
            // (A single-colour font has nothing to make more vivid.)
            glow: if set.coloured() && env.settings.pointer.enabled { env.settings.pointer.glow.min(10) } else { 0 },
        };
        if !spec.fonts.contains(&true) || spec.px_per_em <= 0.0 {
            return; // nothing to draw with yet: placeholders
        }
        if block.sheet.as_ref().is_some_and(|s| s.spec == spec) || block.drawing(&spec) {
            return;
        }
        // While the size keeps changing (a window being resized), the sheet it has is scaled.
        if block.sheet.as_ref().is_some_and(|s| s.spec.same_but_size(&spec)) && now - self.px_changed < RESIZE_SETTLE {
            ctx.request_repaint_after_secs(RESIZE_SETTLE as f32);
            return;
        }
        // What was being drawn for it is out of date.
        block.cancel_job();
        let cancelled = Arc::new(AtomicBool::new(false));
        block.job = Some((spec.clone(), cancelled.clone()));
        block.asked = web_time::Instant::now();
        let input = JobInput {
            cancelled,
            key: block.key,
            spec,
            requests: blocks::requests(&block.page, env.chapters, env.bismillah),
            fonts: vec![page_font, first_font],
            fallback,
            ornaments,
        };
        let mut drawing = blocks::Drawing::new(input);
        task::spawn_steps(task::Pool::Draw, ctx, &self.sheets.0, move |deadline| drawing.step(deadline));
    }

    /// The reading position: the first word of the first line whose middle is
    /// below the top of the viewport.
    pub(super) fn track_position(&mut self, m: &Metrics) {
        let view_top = self.scroll - self.top_height;
        let mut top = 0.0;
        for block in &self.blocks {
            let block_top = top;
            top += block.height;
            if top <= view_top {
                continue;
            }
            let Some(placed) = &block.placed else { return };
            let line = placed.rows.iter().filter(|r| r.is_text()).find_map(|row| {
                let word = row.words().first()?;
                (block_top + row.y + word.y + m.line_height() / 2.0 > view_top + 4.0).then_some((row.item, word.cell?.1))
            });
            let Some((item, index)) = line else { continue };
            let cell = match &block.page.items[item] {
                Item::Line { cells, .. } | Item::Flow { cells, .. } => cells.get(index),
                _ => None,
            };
            if let Some(cell) = cell
                && let Some(at) = WordRef::parse(&cell.location)
            {
                let page = cell.page.or(block.page.page).unwrap_or(0);
                self.position = Some(Position { at, page, top: self.scroll < 8.0 });
            }
            return;
        }
    }

    /// Free the sheets, placements and fonts of pages far from the viewport.
    pub(super) fn release_far(&mut self, env: &mut Env<'_>, view: f32) {
        let view_top = self.scroll - self.top_height;
        let set = FontSet::of(env.settings.mushaf_font);
        // Page 1's font (a few dozen kB) draws every Bismillah line and the voice pointer's preview: kept.
        let mut keep: Vec<u16> = vec![1];
        let mut top = 0.0;
        for block in &mut self.blocks {
            let bottom = top + block.height;
            if bottom < view_top - 2.5 * view || top > view_top + 3.5 * view {
                block.release();
            } else {
                keep.extend(block.page.page);
            }
            top = bottom;
        }
        env.fonts.retain_pages(|s, page| s == set && keep.contains(&page));
    }

    /// With `QURAN_EG_STATS` set: once a second, what the view holds in memory.
    pub(super) fn report(&mut self, ctx: &egui::Context, env: &Env<'_>) {
        static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        let now = ctx.input(|i| i.time);
        if !*ON.get_or_init(|| std::env::var_os("QURAN_EG_STATS").is_some()) || now - self.reported < 1.0 {
            return;
        }
        self.reported = now;
        let sheets: Vec<[usize; 2]> = self.blocks.iter().filter_map(|b| b.sheet.as_ref()).map(|s| s.texture.size()).collect();
        let bytes: usize = sheets.iter().map(|s| s[0] * s[1] * 4).sum();
        eprintln!(
            "stats: {} verses, {} pages, {} placed, {} sheets ({:.1} MB), {} page fonts, scroll {:.0}",
            self.feed.verses.len(),
            self.blocks.len(),
            self.blocks.iter().filter(|b| b.placed.is_some()).count(),
            sheets.len(),
            bytes as f64 / 1_048_576.0,
            env.fonts.loaded_pages(),
            self.scroll,
        );
    }
}
