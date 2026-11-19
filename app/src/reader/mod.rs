//! A reader view (surah, juz or page): verses loaded 50 at a time and drawn as
//! 15-line mushaf pages in one scrolling column.
//!
//! - `blocks`: per-page state (height, placement, sprite sheet) and sheet jobs.
//! - `place`: the geometry of a page.
//! - `paint`: drawing a placed page.
//! - `scroll`: which pages are near the viewport, and keeping the top line in
//!   place while pages around it change height.
//! - `chrome`: what surrounds the pages (notices, navigation, loading state).

mod blocks;
mod chrome;
mod paint;
mod place;
mod scroll;
#[cfg(test)]
mod page_check;
#[cfg(test)]
mod tests;
mod verses;

use std::hash::{Hash, Hasher};
use std::sync::mpsc::{Receiver, Sender, channel};

use egui::{Rect, Sense, Ui, pos2, vec2};
use quran_core::api::{Chapter, Client, Juz, Verse, VerseQuery, VersesResponse};
use quran_core::feed::{Feed, issue_messages};
use quran_core::mushaf_layout::build_layout;
use quran_core::settings::{self, Settings, WbwDisplay};
use quran_core::view::{Location, Position, View, WordRef};

use self::blocks::{Block, SheetResult, block_key};
use self::place::Metrics;
use crate::fonts::FontStore;
use crate::pointer::Lens;
use crate::task::{self, Poll};
use crate::theme::Colors;

/// Padding of the content column (quran-dx's `.site-main`).
pub const COLUMN_PAD: f32 = 19.2;
/// Sheets are not drawn larger than this many pixels per em (bigger text is scaled up).
const MAX_PX_PER_EM: f32 = 192.0;
/// A sheet is redrawn at a new size once the size has stayed put this long.
const RESIZE_SETTLE: f64 = 0.15;

/// What a view needs from the app.
pub struct Env<'a> {
    pub settings: &'a Settings,
    pub colors: &'a Colors,
    pub fonts: &'a mut FontStore,
    pub chapters: &'a [Chapter],
    pub juzs: &'a [Juz],
    pub client: &'a Client,
    /// Verse 1:1, whose words draw the Bismillah lines.
    pub bismillah: Option<&'a Verse>,
    /// Where the reading pointer makes colours more vivid, if it glows.
    pub lens: Option<Lens>,
}

/// Where a view draws: the whole area under the header, and the content column in it.
#[derive(Clone, Copy)]
struct Space {
    area: Rect,
    column: Rect,
}

/// The sizes every page shares, for a column this wide and the widest line seen
/// so far (at scale 1). All pages follow that line: if it doesn't fit the column,
/// everything is scaled down together, and every page's lines are as wide as it.
fn metrics_for(s: &Settings, muted: egui::Color32, column: f32, widest: Option<f32>) -> Metrics {
    let fit = widest.filter(|w| *w > column).map_or(1.0, |w| column / w);
    let em = settings::mushaf_em(s.arabic_font_scale) * fit;
    let wbw = &s.word_by_word;
    let tooltip = wbw.display == WbwDisplay::Tooltip;
    let meanings = (wbw.show_translation || wbw.show_transliteration) && !tooltip;
    let word_gap = settings::word_gap(s.word_spacing);
    Metrics {
        fit,
        em,
        line_width: widest.map_or(column, |w| (w * fit).min(column)),
        word_gap: em * (word_gap + if meanings { 0.18 } else { 0.0 }),
        loose_gap: em * (word_gap + if meanings { 0.18 } else { 0.1 }),
        line_gap: em * settings::line_gap(s.line_spacing),
        meaning_gap: em * settings::meaning_gap(s.meaning_spacing),
        meaning_font: settings::meaning_font_px(s.translation_font_scale) * fit,
        meaning_max_width: settings::MEANING_MAX_WIDTH * fit,
        meanings,
        translation: wbw.show_translation,
        transliteration: wbw.show_transliteration,
        tooltip,
        muted,
    }
}

type Fetched = (u64, u32, Result<VersesResponse, String>);

pub struct Reader {
    pub view: View,
    /// Where the reader is (the first word of the top line), once known.
    pub position: Option<Position>,
    feed: Feed,
    query: VerseQuery,
    fetch: Option<Receiver<Fetched>>,
    /// Bumped on reload so responses to stale requests are dropped.
    generation: u64,
    error: Option<String>,
    blocks: Vec<Block>,
    /// The integrity notice: its lines, and whether reloading could help.
    notice: (Vec<String>, bool),
    /// Number of ajzāʾ known when the notice was made (a juz view needs them).
    checked_juzs: usize,
    /// Width of the content column, as of the last frame.
    column: f32,
    /// Scroll offset from the top of the view's content.
    scroll: f32,
    /// A position to open at, until its verse has loaded.
    pending: Option<WordRef>,
    /// Heights of what is above and below the pages, as measured last frame.
    top_height: f32,
    bottom_height: f32,
    /// Hash of the settings the layout depends on.
    style: u64,
    /// Sheet size wanted last frame, and when it last changed.
    px_per_em: f32,
    px_changed: f64,
    sheets: (Sender<Option<SheetResult>>, Receiver<Option<SheetResult>>),
    sheet_version: u64,
    /// Height of a mushaf line at the current scale (the pointer's size follows it).
    pub line_height: f32,
    /// When the memory statistics were last printed.
    reported: f64,
    /// Footnotes and copy state of the verse translations.
    notes: verses::Notes,
}

impl Reader {
    /// Open `view`, at `at` if given, and start loading.
    pub fn open(view: View, at: Option<WordRef>, ctx: &egui::Context, env: &Env<'_>) -> Option<Self> {
        let mut reader = Self::closed(view, Feed::new(view.source()?), env);
        reader.pending = at;
        reader.load_more(ctx, env.client);
        Some(reader)
    }

    /// Scroll to a position in this view (loading up to it if need be).
    pub fn go_to(&mut self, at: Option<WordRef>) {
        if at.is_some() {
            self.pending = at;
        }
    }

    /// A view that has not asked for anything yet.
    fn closed(view: View, feed: Feed, env: &Env<'_>) -> Self {
        Self {
            view,
            position: None,
            feed,
            query: env.settings.verse_query(),
            fetch: None,
            generation: 0,
            error: None,
            blocks: Vec::new(),
            notice: (Vec::new(), false),
            checked_juzs: 0,
            column: settings::content_width_px(env.settings.content_width) as f32 - 2.0 * COLUMN_PAD,
            scroll: 0.0,
            pending: None,
            top_height: 0.0,
            bottom_height: 0.0,
            style: 0,
            px_per_em: 0.0,
            px_changed: 0.0,
            sheets: channel(),
            sheet_version: 0,
            line_height: settings::mushaf_em(env.settings.arabic_font_scale) * settings::MUSHAF_LINE,
            reported: 0.0,
            notes: verses::Notes::default(),
        }
    }

    /// Where the reader is, as a location to save or to open in another view.
    /// Until the place it was opened at has loaded, that place.
    pub fn location(&self) -> Location {
        self.view.at(self.pending.or(self.position.filter(|p| !p.top).map(|p| p.at)))
    }

    /// Load everything again (the verse query changed, or a retry), staying at the place being read.
    pub fn reload(&mut self, ctx: &egui::Context, env: &Env<'_>) {
        self.generation += 1;
        self.pending = self.location().at.or(self.pending);
        self.feed = Feed::new(self.feed.source);
        self.query = env.settings.verse_query();
        self.fetch = None;
        self.error = None;
        self.blocks.clear();
        self.notice = (Vec::new(), false);
        self.scroll = 0.0;
        self.load_more(ctx, env.client);
    }

    fn loading(&self) -> bool {
        self.fetch.is_some()
    }

    fn load_more(&mut self, ctx: &egui::Context, client: &Client) {
        let Some(page) = self.feed.next_page else { return };
        if self.loading() {
            return;
        }
        self.error = None;
        let (source, generation) = (self.feed.source, self.generation);
        self.fetch = Some(crate::net::call(ctx, source.call(client, &self.query, page), move |answer| {
            let verses = answer.and_then(|response| source.accept(page, response));
            (generation, page, verses.map_err(|e| e.to_string()))
        }));
    }

    /// Take in what the worker threads have finished.
    fn poll(&mut self, ctx: &egui::Context, env: &Env<'_>) {
        match self.fetch.as_ref().map(task::poll) {
            Some(Poll::Done((generation, page, result))) => {
                self.fetch = None;
                if generation == self.generation {
                    match result {
                        Ok(response) => {
                            self.feed.merge(page, response);
                            self.rebuild(env);
                            crate::stats::milestone("first-verses");
                            // A mushaf page is one unit: its other listing is fetched now.
                            if self.feed.source.loads_at_once() {
                                self.load_more(ctx, env.client);
                            }
                        }
                        Err(e) => self.error = Some(e),
                    }
                }
            }
            Some(Poll::Lost) => {
                self.fetch = None;
                self.error = Some("the request ended unexpectedly".into());
            }
            Some(Poll::Pending) | None => {}
        }
        // The juz list arriving later completes the check of a juz view.
        if self.checked_juzs != env.juzs.len() && !self.feed.verses.is_empty() {
            self.check(env);
        }
        while let Ok(result) = self.sheets.1.try_recv() {
            let Some(result) = result else { continue };
            if let Some(block) = self.blocks.iter_mut().find(|b| b.key == result.key) {
                self.sheet_version += 1;
                block.accept(ctx, result, self.sheet_version);
            }
            crate::stats::milestone("first-sheet");
        }
    }

    fn check(&mut self, env: &Env<'_>) {
        let issues = self.feed.issues(&self.view.expected(env.juzs));
        if !issues.is_empty() {
            let list: Vec<String> = issues.iter().map(ToString::to_string).collect();
            eprintln!("quran-eg: verse integrity issues ({}): {}", self.view, quran_core::util::short_list(&list, 20));
        }
        self.notice = issue_messages(&issues);
        self.checked_juzs = env.juzs.len();
    }

    /// New verses arrived: regroup them into pages, keeping the state of the
    /// pages that are already there.
    fn rebuild(&mut self, env: &Env<'_>) {
        self.check(env);
        let layout = build_layout(&self.feed.verses, self.view.trim());
        let mut old: Vec<Block> = std::mem::take(&mut self.blocks);
        let estimate = self.metrics(env);
        self.blocks = layout
            .pages
            .iter()
            .enumerate()
            .map(|(i, page)| {
                let key = block_key(&layout.pages, i);
                let height = estimate.estimate(page);
                match old.iter().position(|b| b.key == key) {
                    Some(at) => {
                        let mut block = old.swap_remove(at);
                        let cells = blocks::cell_count(page);
                        if block.cells != cells {
                            // The page grew: its geometry is redone (its sheet keeps serving the words it has).
                            block.cells = cells;
                            block.placed = None;
                            block.height = height.max(block.height);
                        }
                        block.page = page.clone();
                        block
                    }
                    None => {
                        let mut block = Block::new(key, page.clone(), height);
                        if env.settings.show_page_translation {
                            block.below = scroll::below_estimate(&block, env);
                            block.height += block.below;
                        }
                        block
                    }
                }
            })
            .collect();
    }

    /// The widest line any page seen so far needs, at scale 1.
    fn widest(&self) -> Option<f32> {
        self.blocks.iter().filter_map(|b| b.natural).reduce(f32::max)
    }

    /// The sizes every page shares (see [`metrics_for`]).
    fn metrics(&self, env: &Env<'_>) -> Metrics {
        metrics_for(env.settings, env.colors.muted, self.column, self.widest())
    }

    #[cfg(test)]
    fn metrics_now(&self, settings: &Settings, colors: &Colors) -> Metrics {
        let mut fonts = FontStore::new(&egui::Context::default(), Vec::new(), None);
        let client = Client::new("http://127.0.0.1:9");
        self.metrics(&Env { settings, colors, fonts: &mut fonts, chapters: &[], juzs: &[], client: &client, bismillah: None, lens: None })
    }

    /// The settings changed in a way that changes line widths: what was
    /// measured under the old ones no longer says how wide the widest line is.
    fn restyle(&mut self, env: &Env<'_>) {
        let s = env.settings;
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        (s.arabic_font_scale, s.translation_font_scale, s.word_spacing, s.mushaf_font as u8).hash(&mut hasher);
        (s.word_by_word.show_translation, s.word_by_word.show_transliteration, s.word_by_word.display as u8).hash(&mut hasher);
        let style = hasher.finish();
        if self.style != style {
            self.style = style;
            for block in &mut self.blocks {
                block.natural = None;
            }
        }
    }

    /// Draw the view into what is left of `ui`. Returns a location to open, if
    /// one of its links was clicked.
    pub fn ui(&mut self, ui: &mut Ui, env: &mut Env<'_>) -> Option<Location> {
        let ctx = ui.ctx().clone();
        self.poll(&ctx, env);
        let area = ui.available_rect_before_wrap();
        ui.allocate_rect(area, Sense::hover());
        let width = (settings::content_width_px(env.settings.content_width) as f32).min(area.width());
        let column = Rect::from_center_size(area.center(), vec2(width - 2.0 * COLUMN_PAD, area.height()));
        self.column = column.width().max(120.0);
        self.restyle(env);
        let metrics = self.metrics(env);
        self.line_height = metrics.line_height();
        let view = area.height();

        self.open_pending(&ctx, env, &metrics);
        self.scroll_input(ui, area);
        self.place_near(&ctx, env, &metrics, view);
        let pages: f32 = self.blocks.iter().map(|b| b.height).sum();
        let total = self.top_height + pages + self.bottom_height;
        self.scroll = self.scroll.clamp(0.0, (total - view).max(0.0));
        // A new scale shows up in the widest line only after placing: go again.
        if self.metrics(env) != metrics {
            ctx.request_repaint();
        }

        let mut open = None;
        let space = Space { area, column };
        let top = area.top() - self.scroll;
        self.top_chrome(ui, env, space, top, &mut open);
        let pages_top = top + self.top_height;
        self.paint_pages(ui, env, &metrics, space, pages_top, &mut open);
        self.bottom_chrome(ui, env, space, pages_top + pages, &mut open);
        if let Some(offset) = crate::kit::scroll_bar(ui, env.colors, area, self.scroll, total) {
            self.scroll = offset;
        }

        self.track_position(&metrics);
        self.release_far(env, view);
        self.report(&ctx, env);
        // Load the next verses before the reader reaches the end of what is there.
        if self.scroll + view * 2.5 > total && self.error.is_none() && self.pending.is_none() {
            self.load_more(&ctx, env.client);
        }
        open
    }

    fn paint_pages(
        &mut self,
        ui: &mut Ui,
        env: &Env<'_>,
        metrics: &Metrics,
        space: Space,
        pages_top: f32,
        open: &mut Option<Location>,
    ) {
        let Space { area, column } = space;
        let painter = ui.painter_at(area);
        let hover = ui.rect_contains_pointer(area).then(|| ui.input(|i| i.pointer.hover_pos())).flatten();
        let canvas = paint::Canvas {
            painter: &painter,
            colors: env.colors,
            metrics,
            chapters: env.chapters,
            pixels_per_point: ui.ctx().pixels_per_point(),
            clip: area,
            hover,
            tip: Default::default(),
            lens: env.lens,
            vivid: Default::default(),
        };
        let left = (column.center().x - metrics.line_width / 2.0).round();
        let translations = env.settings.show_page_translation;
        let mut shift = 0.0;
        let mut y = pages_top;
        for i in 0..self.blocks.len() {
            let top = y;
            y += self.blocks[i].height;
            let block = &self.blocks[i];
            let Some(placed) = block.placed.as_ref().filter(|_| top < area.bottom() && y > area.top()) else { continue };
            let pill = canvas.page(pos2(left, top), block, placed);
            // The page number opens the page on its own.
            if let Some(page) = block.page.page.filter(|p| pill.intersects(area) && self.view != View::Page(*p)) {
                let response = ui.interact(pill, ui.scope_id().with(("page-pill", block.key)), Sense::click());
                if response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    *open = Some(View::Page(page).at(None));
                }
            }
            if !translations {
                continue;
            }
            // The page's verses with their translations, in a column of their own under it.
            let under = top + placed.height;
            let width = column.width().min(verses::MAX_WIDTH);
            let rect = Rect::from_min_size(pos2(column.center().x - width / 2.0, under), vec2(width, 100_000.0));
            let list: Vec<&Verse> =
                block.page.verse_keys.iter().filter_map(|k| self.feed.verses.iter().find(|v| &v.verse_key == k)).collect();
            let builder = egui::UiBuilder::new().max_rect(rect).id_salt(("translations", block.key));
            let mut child = ui.new_child(builder.layout(egui::Layout::top_down(egui::Align::Min)));
            child.set_clip_rect(area);
            if let Some(to) = verses::show(&mut child, env, &mut self.notes, &list) {
                *open = Some(to);
            }
            let change = child.min_rect().height() + 20.0 - block.below;
            if change.abs() > 0.5 {
                let block = &mut self.blocks[i];
                block.below += change;
                block.height += change;
                y += change;
                // What grew above the top of the window must not push the line being read.
                if under < area.top() {
                    shift += change;
                }
                ui.ctx().request_repaint();
            }
        }
        self.scroll += shift;
        canvas.finish(ui.ctx());
    }
}
