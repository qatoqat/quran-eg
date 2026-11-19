//! Search: the header's search box with its suggestions (jump shortcuts, and
//! verse matches from the API after a short pause in typing), and the results
//! view with highlighted matches and paging.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::Receiver;

use egui::{Align, Color32, ColorImage, FontId, Key, Layout, Mesh, Rect, RichText, Sense, Shape, Stroke, StrokeKind, TextureHandle, TextureOptions, Ui, pos2, vec2};
use quran_core::api::{self, Chapter, Client, SearchResult};
use quran_core::highlight;
use quran_core::nav::{self, NavSuggestion, describe_error, plural, verse_view};
use quran_core::settings::{self, LANGUAGE};
use quran_core::view::Location;

use crate::arabic::ArabicFont;
use crate::glyphs::Palette;
use crate::kit::{self, Arrow, Pill};
use crate::reader::{COLUMN_PAD, Env};
use crate::sheet::{self, Request, Sprite, Text};
use crate::task::{self, Poll, Pool};
use crate::theme::Colors;

/// Typing pauses this long before verses are searched for.
const DEBOUNCE: f64 = 0.3;
const SUGGESTIONS: u32 = 5;
const PER_PAGE: u32 = 20;

type Found = (String, u32, Result<api::Search, String>);

fn ask(ctx: &egui::Context, client: &Client, query: &str, size: u32, page: u32) -> Receiver<Found> {
    let query = query.trim().to_string();
    let call = client.search_call(&query, size, page, LANGUAGE);
    crate::net::call(ctx, call, move |answer| (query, page, answer.map_err(|e| describe_error(&e))))
}

/// The Arabic text of a list of verses, word by word, in one texture.
struct Words {
    texture: TextureHandle,
    sprites: HashMap<u64, Sprite>,
}

#[derive(Default)]
struct WordSheet {
    words: Option<Words>,
    job: Option<Receiver<(ColorImage, HashMap<u64, Sprite>)>>,
    /// What the sheet was asked for: the verses and the size.
    stamp: u64,
}

fn word_key(verse: usize, word: usize) -> u64 {
    (verse as u64) << 20 | word as u64
}

/// The words of a result's Arabic text, with their highlights.
fn arabic_words(result: &SearchResult) -> Vec<(String, bool)> {
    highlight::words(result.highlighted.as_deref().filter(|h| !h.trim().is_empty()).unwrap_or(&result.text))
}

impl WordSheet {
    /// See to it that the sheet holds the words of `results` at `size` points.
    fn ensure(&mut self, ui: &Ui, font: Option<Arc<ArabicFont>>, results: &[SearchResult], size: f32, stamp: u64) {
        if let Some(Poll::Done((image, sprites))) = self.job.as_ref().map(task::poll) {
            self.job = None;
            let texture = ui.ctx().load_texture("search-words", image, TextureOptions::LINEAR);
            self.words = Some(Words { texture, sprites });
        }
        let ppp = ui.ctx().pixels_per_point();
        let stamp = stamp ^ u64::from((size * ppp * 16.0) as u32) << 40;
        let Some(font) = font.filter(|_| self.stamp != stamp) else { return };
        self.stamp = stamp;
        self.words = None;
        let requests: Vec<Request> = results
            .iter()
            .enumerate()
            .flat_map(|(i, r)| arabic_words(r).into_iter().enumerate().map(move |(j, (word, _))| (word_key(i, j), word)))
            .map(|(key, word)| Request { key, glyph: None, text: Some(Text::plain(word, 1.0)) })
            .collect();
        self.job = Some(task::run(Pool::Draw, ui.ctx(), move || {
            let job = sheet::Job { requests, fonts: Vec::new(), fallback: Some(font), ornaments: None, px_per_em: size * ppp, palette: Palette::Light };
            let sheet = sheet::build(job);
            (ColorImage::from_rgba_premultiplied([sheet.width, sheet.height], &sheet.rgba), sheet.sprites)
        }));
    }

    /// Draw verse `index`'s words right to left from the top right of `rect`,
    /// wrapping (at most `max_lines` lines; an ellipsis marks what was cut).
    /// Returns the height used.
    fn paragraph(&self, ui: &Ui, colors: &Colors, rect: Rect, verse: (usize, &[(String, bool)]), size: f32, max_lines: usize) -> f32 {
        let (index, words) = verse;
        let line = size * 1.9;
        let Some(sheet) = &self.words else {
            // The text font is on its way (or could not be had): say so rather than show nothing.
            let note = if self.job.is_some() { "Loading the Arabic text…" } else { "(The Arabic text needs the text font, which is not loaded.)" };
            ui.painter().text(rect.right_top() + vec2(0.0, line / 2.0), egui::Align2::RIGHT_CENTER, note, FontId::proportional(13.0), colors.muted);
            return line;
        };
        let gap = size * 0.28;
        let mut mesh = Mesh::with_texture(sheet.texture.id());
        let (mut right, mut row) = (rect.right(), 0usize);
        for (j, (_, lit)) in words.iter().enumerate() {
            let Some(sprite) = sheet.sprites.get(&word_key(index, j)) else { continue };
            let width = sprite.advance * size;
            if right - width < rect.left() && right < rect.right() {
                row += 1;
                right = rect.right();
                if row >= max_lines {
                    let at = pos2(rect.left(), rect.top() + (row as f32 - 0.5) * line);
                    ui.painter().text(at, egui::Align2::LEFT_CENTER, "…", FontId::proportional(size * 0.7), colors.muted);
                    break;
                }
            }
            let baseline = rect.top() + row as f32 * line + line * 0.68;
            if *lit {
                let mark = Rect::from_min_max(pos2(right - width - 2.0, baseline - size * 0.95), pos2(right + 2.0, baseline + size * 0.45));
                ui.painter().rect_filled(mark, 3, Colors::faint(colors.accent, 0.28));
            }
            let mut pen = right;
            for part in &sprite.parts {
                pen -= part.advance * size;
                let image = Rect::from_min_size(pos2(pen + part.left * size, baseline - part.top * size), vec2(part.size[0], part.size[1]) * size);
                mesh.add_rect_with_uv(image, Rect::from_min_max(pos2(part.uv[0], part.uv[1]), pos2(part.uv[2], part.uv[3])), colors.fg);
            }
            right -= width + gap;
        }
        ui.painter().add(Shape::mesh(mesh));
        (row.min(max_lines - 1) + 1) as f32 * line
    }

    /// Height `paragraph` will use for these words in `width`.
    fn height(&self, words: usize, index: usize, size: f32, width: f32) -> f32 {
        let line = size * 1.9;
        let Some(sheet) = &self.words else { return line };
        let (mut right, mut rows) = (width, 1);
        for j in 0..words {
            let Some(sprite) = sheet.sprites.get(&word_key(index, j)) else { continue };
            let advance = sprite.advance * size;
            if right - advance < 0.0 && right < width {
                rows += 1;
                right = width;
            }
            right -= advance + size * 0.28;
        }
        rows as f32 * line
    }
}

/// A translation snippet with its matches marked.
fn snippet(colors: &Colors, html: &str, size: f32, color: Color32) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    for segment in highlight::segments(html) {
        let mut format = egui::TextFormat::simple(FontId::proportional(size), color);
        if segment.highlighted {
            format.background = Colors::faint(colors.accent, 0.28);
        }
        job.append(&segment.text, 0.0, format);
    }
    job
}

/// A small outlined label: a verse key, "Page 3".
fn badge(ui: &Ui, colors: &Colors, at: egui::Pos2, text: &str, muted: bool) -> Rect {
    let (ink, line) = if muted { (colors.muted, colors.border) } else { (colors.accent, Colors::faint(colors.accent, 0.45)) };
    let galley = ui.painter().layout_no_wrap(text.to_owned(), FontId::proportional(12.5), ink);
    let rect = Rect::from_min_size(at, vec2((galley.size().x + 16.0).max(51.0), 22.0));
    ui.painter().rect_stroke(rect, 6, Stroke::new(1.0, line), StrokeKind::Inside);
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, ink);
    rect
}

enum Verses {
    Idle,
    Loading,
    Ready(Vec<SearchResult>),
    Failed(String),
}

/// One row of the dropdown.
enum Option_ {
    Nav(NavSuggestion),
    Verse(usize),
    All,
}

pub struct SearchBox {
    pub open: bool,
    query: String,
    /// The box was just opened and should take the keyboard.
    focus: bool,
    dropdown: bool,
    active: Option<usize>,
    /// When the query last changed, and the query the verses are (being) fetched for.
    typed: f64,
    asked: String,
    job: Option<Receiver<Found>>,
    verses: Verses,
    words: WordSheet,
}

/// What the search box asks the app to do.
pub enum Go {
    To(Location),
    /// Show all results for this query.
    Results(String),
}

impl Default for SearchBox {
    fn default() -> Self {
        Self {
            open: false,
            query: String::new(),
            focus: false,
            dropdown: false,
            active: None,
            typed: 0.0,
            asked: String::new(),
            job: None,
            verses: Verses::Idle,
            words: WordSheet::default(),
        }
    }
}

impl SearchBox {
    pub fn toggle(&mut self) {
        self.open = !self.open;
        self.focus = self.open;
        self.dropdown = self.open;
    }

    pub fn close(&mut self) {
        self.open = false;
        self.dropdown = false;
        self.active = None;
    }

    /// Show the query of the results view in the box.
    pub fn set_query(&mut self, query: &str) {
        self.query = query.to_string();
    }

    fn fetch(&mut self, ui: &Ui, client: &Client) {
        let now = ui.input(|i| i.time);
        match self.job.as_ref().map(task::poll) {
            Some(Poll::Done((query, _, found))) => {
                self.job = None;
                if query == self.query.trim() {
                    self.verses = match found {
                        Ok(search) => Verses::Ready(search.results),
                        Err(e) => Verses::Failed(e),
                    };
                }
            }
            Some(Poll::Lost) => {
                self.job = None;
                self.verses = Verses::Failed("Search failed: the request ended unexpectedly.".into());
            }
            _ => {}
        }
        let query = self.query.trim();
        if query.is_empty() {
            self.verses = Verses::Idle;
            self.asked.clear();
        } else if query != self.asked && self.job.is_none() {
            if now - self.typed >= DEBOUNCE {
                self.asked = query.to_string();
                self.verses = Verses::Loading;
                self.job = Some(ask(ui.ctx(), client, query, SUGGESTIONS, 1));
            } else {
                ui.ctx().request_repaint_after_secs(DEBOUNCE as f32);
            }
        }
    }

    /// The text field (in `field`) and, under it, the suggestions.
    pub fn ui(&mut self, ui: &mut Ui, env: &mut Env<'_>, field: Rect) -> Option<Go> {
        let colors = env.colors;
        self.fetch(ui, env.client);
        let id = ui.scope_id().with("search-box");
        let edit = egui::TextEdit::singleline(&mut self.query)
            .id(id)
            .hint_text("Search, or jump to 2:255, page 3, Al-Kahf…")
            .margin(egui::Margin::symmetric(10, 7))
            .desired_width(field.width() - 20.0);
        let response = ui.put(field, edit);
        if std::mem::take(&mut self.focus) {
            response.request_focus();
        }
        if response.changed() {
            self.typed = ui.input(|i| i.time);
            self.dropdown = true;
            self.active = None;
        }
        if response.gained_focus() {
            self.dropdown = true;
        }

        let query = self.query.trim().to_string();
        let mut options: Vec<Option_> = nav::parse(&query, env.chapters).into_iter().map(Option_::Nav).collect();
        let jumps = options.len();
        if let Verses::Ready(results) = &self.verses {
            options.extend((0..results.len()).map(Option_::Verse));
        }
        if !query.is_empty() {
            options.push(Option_::All);
        }
        let expanded = self.dropdown && !query.is_empty();
        let active = self.active.filter(|i| expanded && *i < options.len());

        let go_to = |option: &Option_, verses: &Verses| match option {
            Option_::Nav(suggestion) => Some(Go::To(suggestion.view().at(None))),
            Option_::Verse(i) => match verses {
                Verses::Ready(results) => verse_view(&results[*i].verse_key).map(|v| Go::To(v.at(None))),
                _ => None,
            },
            Option_::All => Some(Go::Results(query.clone())),
        };
        let mut go = None;

        // Keys: arrows walk the options, Enter takes one (or all results), Escape backs out.
        let (down, up, enter, escape) = ui.input(|i| {
            (i.key_pressed(Key::ArrowDown), i.key_pressed(Key::ArrowUp), i.key_pressed(Key::Enter), i.key_pressed(Key::Escape))
        });
        let focused = response.has_focus() || response.lost_focus();
        if focused && (down || up) && !options.is_empty() {
            let count = options.len();
            self.dropdown = true;
            self.active = Some(match (active, down) {
                (None, true) => 0,
                (None, false) => count - 1,
                (Some(i), true) => (i + 1) % count,
                (Some(i), false) => (i + count - 1) % count,
            });
        }
        if response.lost_focus() && enter {
            go = match active.and_then(|i| options.get(i)) {
                Some(option) => go_to(option, &self.verses),
                None if !query.is_empty() => Some(Go::Results(query.clone())),
                None => None,
            };
            if go.is_none() {
                response.request_focus();
            }
        } else if response.lost_focus() && escape && expanded {
            self.dropdown = false;
            self.active = None;
            response.request_focus();
        } else if response.lost_focus() && escape {
            self.close();
        }

        if expanded {
            let size = 20.0;
            if let Verses::Ready(results) = &self.verses {
                let stamp = results.iter().fold(7u64, |h, r| h.wrapping_mul(31).wrapping_add(r.verse_id.unwrap_or(0).into()));
                self.words.ensure(ui, env.fonts.fallback(), results, size, stamp);
            }
            let screen = ui.ctx().content_rect();
            let width = 544.0f32.min(screen.width() - 22.0);
            let left = (field.center().x - width / 2.0).clamp(screen.left() + 11.0, (screen.right() - 11.0 - width).max(screen.left() + 11.0));
            let area = egui::Area::new(id.with("dropdown")).order(egui::Order::Foreground).fixed_pos(pos2(left, field.bottom() + 6.0));
            let over = area
                .constrain(false)
                .show(ui.ctx(), |ui| {
                    let frame = egui::Frame::new()
                        .fill(colors.card)
                        .stroke(Stroke::new(1.0, colors.border))
                        .corner_radius(10)
                        .shadow(egui::Shadow { offset: [0, 10], blur: 30, spread: 0, color: Color32::from_black_alpha(46) });
                    frame.show(ui, |ui| {
                        ui.set_width(width);
                        ui.spacing_mut().item_spacing.y = 0.0;
                        {
                            ui.add_space(5.0);
                            for (i, option) in options.iter().enumerate() {
                                if i == 0 && jumps > 0 {
                                    group(ui, colors, "Jump to");
                                }
                                if i == jumps && matches!(option, Option_::Verse(_)) {
                                    group(ui, colors, "Verses");
                                }
                                if self.option(ui, colors, option, active == Some(i), &query) {
                                    go = go_to(option, &self.verses);
                                }
                            }
                            let status = match &self.verses {
                                Verses::Loading => Some(("Searching verses…".to_string(), colors.muted)),
                                Verses::Failed(message) => Some((message.clone(), colors.danger)),
                                Verses::Ready(results) if results.is_empty() => Some(("No matching verses.".to_string(), colors.muted)),
                                _ => None,
                            };
                            if let Some((text, ink)) = status {
                                egui::Frame::new().inner_margin(egui::Margin::symmetric(14, 8)).show(ui, |ui| {
                                    ui.label(RichText::new(text).size(13.6).color(ink));
                                });
                            }
                            ui.add_space(5.0);
                        }
                    });
                })
                .response
                .contains_pointer();
            // A click elsewhere closes the suggestions (the box stays for the next query).
            if response.lost_focus() && !enter && !escape && !over && go.is_none() {
                self.dropdown = false;
            }
        } else if response.lost_focus() && !enter && !escape && query.is_empty() {
            self.close();
        }
        if go.is_some() {
            self.dropdown = false;
            self.active = None;
        }
        // A jump is done with: the box starts empty next time. (A search keeps its query.)
        if matches!(go, Some(Go::To(_))) {
            self.query.clear();
        }
        go
    }

    /// One suggestion; returns whether it was clicked.
    fn option(&self, ui: &mut Ui, colors: &Colors, option: &Option_, active: bool, query: &str) -> bool {
        let width = ui.available_width();
        let (pad, badge_width) = (14.0, 78.0);
        let text_width = width - 2.0 * pad - badge_width;
        // Height first: one line, or for a verse its Arabic line and up to two lines of translation.
        let translation = match (option, &self.verses) {
            (Option_::Verse(i), Verses::Ready(results)) => results[*i].translations.first().map(|t| {
                let mut job = snippet(colors, &t.text, 13.6, colors.muted);
                job.wrap = egui::text::TextWrapping { max_width: text_width, max_rows: 2, overflow_character: Some('…'), ..Default::default() };
                ui.painter().layout_job(job)
            }),
            _ => None,
        };
        let height = match option {
            Option_::Verse(_) => 20.0 * 1.9 + translation.as_ref().map_or(0.0, |g| g.size().y + 2.0) + 12.0,
            _ => 38.0,
        };
        let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::click());
        if active || response.hovered() {
            ui.painter().rect_filled(rect, 0, Colors::faint(colors.accent, 0.14));
        }
        let at = rect.min + vec2(pad, 8.0);
        let text_left = at.x + badge_width;
        let label = |text: &str| {
            let galley = ui.painter().layout(text.to_owned(), FontId::proportional(14.4), colors.fg, text_width);
            ui.painter().galley(pos2(text_left, rect.center().y - galley.size().y / 2.0), galley, colors.fg);
        };
        match (option, &self.verses) {
            (Option_::Nav(suggestion), _) => {
                badge(ui, colors, at, &suggestion.target.badge(), false);
                label(&suggestion.label);
            }
            (Option_::Verse(i), Verses::Ready(results)) => {
                let result = &results[*i];
                badge(ui, colors, at, &result.verse_key, false);
                let text = Rect::from_min_size(pos2(text_left, rect.top() + 4.0), vec2(text_width, 20.0 * 1.9));
                let used = self.words.paragraph(ui, colors, text, (*i, &arabic_words(result)), 20.0, 1);
                if let Some(galley) = translation {
                    ui.painter().galley(pos2(text_left, text.top() + used + 2.0), galley, colors.muted);
                }
            }
            (Option_::All, _) | (Option_::Verse(_), _) => {
                badge(ui, colors, at, "Search", true);
                label(&format!("Show all results for “{query}”"));
            }
        }
        response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
    }
}

fn group(ui: &mut Ui, colors: &Colors, title: &str) {
    egui::Frame::new().inner_margin(egui::Margin { left: 14, right: 14, top: 7, bottom: 3 }).show(ui, |ui| {
        ui.label(RichText::new(title.to_uppercase()).size(11.5).color(colors.muted));
    });
}

/// The results view for one query.
pub struct Results {
    pub query: String,
    page: u32,
    job: Option<Receiver<Found>>,
    found: Option<Result<api::Search, String>>,
    words: WordSheet,
    /// Scroll back to the top (a new page of results arrived).
    to_top: bool,
}

impl Results {
    pub fn new(ctx: &egui::Context, client: &Client, query: &str) -> Self {
        let query = query.trim().to_string();
        let job = (!query.is_empty()).then(|| ask(ctx, client, &query, PER_PAGE, 1));
        Self { query, page: 1, job, found: None, words: WordSheet::default(), to_top: false }
    }

    fn turn(&mut self, ui: &Ui, client: &Client, page: u32) {
        self.page = page;
        self.job = Some(ask(ui.ctx(), client, &self.query, PER_PAGE, page));
    }

    /// Draw the view. Returns a place to go to, if a result or a shortcut was chosen.
    pub fn ui(&mut self, ui: &mut Ui, env: &mut Env<'_>, chapters: &[Chapter]) -> Option<Location> {
        let colors = env.colors;
        match self.job.as_ref().map(task::poll) {
            Some(Poll::Done((_, page, found))) if page == self.page => {
                self.job = None;
                self.found = Some(found);
                self.to_top = true;
            }
            Some(Poll::Done(_)) => self.job = None,
            Some(Poll::Lost) => {
                self.job = None;
                self.found = Some(Err("Search failed: the request ended unexpectedly.".into()));
            }
            _ => {}
        }
        let pending = self.job.is_some();
        let size = settings::arabic_font_rem(env.settings.arabic_font_scale) * settings::REM * 0.8;
        if let Some(Ok(search)) = &self.found {
            self.words.ensure(ui, env.fonts.fallback(), &search.results, size, u64::from(self.page) << 8 | search.results.len() as u64);
        }
        let mut go = None;
        let mut turn = None;
        let mut scroll = egui::ScrollArea::vertical().auto_shrink(false);
        if std::mem::take(&mut self.to_top) {
            scroll = scroll.vertical_scroll_offset(0.0);
        }
        scroll.show(ui, |ui| {
            let width = (settings::content_width_px(env.settings.content_width) as f32).min(ui.available_width());
            let side = ((ui.available_width() - width) / 2.0).max(0.0) + COLUMN_PAD;
            ui.horizontal_top(|ui| {
                ui.add_space(side);
                ui.vertical(|ui| {
                    ui.set_width((width - 2.0 * COLUMN_PAD).max(120.0));
                    ui.spacing_mut().item_spacing.y = 12.0;
                    ui.add_space(20.0);
                    let title = if self.query.is_empty() { "Search".to_string() } else { format!("Search results for “{}”", self.query) };
                    ui.label(RichText::new(title).size(24.0).color(colors.fg));
                    if self.query.is_empty() {
                        centred(ui, colors, "Type a word or phrase in the search box above, or a verse key such as 2:255.");
                        return;
                    }
                    // Shortcuts the query also means.
                    let jumps = nav::parse(&self.query, chapters);
                    if !jumps.is_empty() {
                        card(ui, colors, |ui| {
                            ui.label(RichText::new("JUMP TO").size(12.0).color(colors.muted));
                            for suggestion in &jumps {
                                let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::click());
                                let tag = badge(ui, colors, rect.min + vec2(0.0, 2.0), &suggestion.target.badge(), false);
                                let ink = if response.hovered() { colors.accent } else { colors.fg };
                                let at = pos2(tag.right() + 10.0, rect.center().y);
                                ui.painter().text(at, egui::Align2::LEFT_CENTER, &suggestion.label, FontId::proportional(15.0), ink);
                                if response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                                    go = Some(suggestion.view().at(None));
                                }
                            }
                        });
                    }
                    match &self.found {
                        None => centred(ui, colors, "Searching…"),
                        Some(Err(message)) => {
                            kit::notice(ui, colors, kit::Notice::Error, |ui| ui.label(message));
                            ui.vertical_centered(|ui| {
                                if kit::pill(ui, colors, "Try again", Pill::Primary).clicked() {
                                    turn = Some(self.page);
                                }
                            });
                        }
                        Some(Ok(search)) if search.results.is_empty() => {
                            centred(ui, colors, &format!("No verses found for “{}”.", search.query));
                            centred(ui, colors, "Try fewer or different words, check the spelling, or search in Arabic.");
                        }
                        Some(Ok(search)) => {
                            let first = (search.current_page.max(1) - 1) * PER_PAGE + 1;
                            let last = first + search.results.len() as u32 - 1;
                            let total = plural(search.total_results, "result", "results");
                            ui.label(RichText::new(format!("Showing {first}–{last} of {total}")).size(14.4).color(colors.muted));
                            // A page being fetched: the old one stays, dimmed.
                            if pending {
                                ui.multiply_opacity(0.55);
                            }
                            for (i, result) in search.results.iter().enumerate() {
                                if let Some(to) = self.result(ui, env, i, result, size) {
                                    go = Some(to);
                                }
                            }
                            if search.total_pages > 1 {
                                let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::hover());
                                let label = format!("Page {} of {}", self.page, search.total_pages);
                                ui.painter().text(row.center(), egui::Align2::CENTER_CENTER, label, FontId::proportional(14.4), colors.muted);
                                let mut side = |layout: Layout, text: &str, arrow: Arrow, to: Option<u32>| {
                                    ui.scope_builder(egui::UiBuilder::new().max_rect(row).layout(layout), |ui| {
                                        ui.add_enabled_ui(to.is_some() && !pending, |ui| {
                                            if kit::arrow_pill(ui, colors, text, Pill::Outline, arrow).clicked() {
                                                turn = to;
                                            }
                                        });
                                    });
                                };
                                side(Layout::left_to_right(Align::Center), "Previous", Arrow::Left, (self.page > 1).then(|| self.page - 1));
                                side(Layout::right_to_left(Align::Center), "Next", Arrow::Right, (self.page < search.total_pages).then(|| self.page + 1));
                            }
                        }
                    }
                    ui.add_space(40.0);
                });
            });
        });
        if let Some(page) = turn {
            self.turn(ui, env.client, page);
        }
        go
    }

    /// One result: its verse key (which opens the verse), the Arabic text and
    /// the translations, with the matches marked.
    fn result(&self, ui: &mut Ui, env: &Env<'_>, index: usize, result: &SearchResult, size: f32) -> Option<Location> {
        let colors = env.colors;
        let mut go = None;
        card(ui, colors, |ui| {
            let (tag, response) = ui.allocate_exact_size(vec2(64.0, 22.0), Sense::click());
            let tag = badge(ui, colors, tag.min, &result.verse_key, false);
            if response.hovered() {
                ui.painter().rect_filled(tag, 6, colors.accent_tint());
            }
            if response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                go = verse_view(&result.verse_key).map(|v| v.at(None));
            }
            let words = arabic_words(result);
            let width = ui.available_width();
            let height = self.words.height(words.len(), index, size, width);
            let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
            if ui.is_rect_visible(rect) {
                self.words.paragraph(ui, colors, rect, (index, &words), size, usize::MAX);
            }
            let text = settings::translation_font_rem(env.settings.translation_font_scale) * settings::REM;
            for translation in &result.translations {
                let y = ui.cursor().top();
                let ends = [pos2(ui.max_rect().left(), y), pos2(ui.max_rect().right(), y)];
                ui.painter().extend(Shape::dashed_line(&ends, Stroke::new(1.0, colors.border), 4.0, 3.0));
                ui.add_space(4.0);
                let mut job = snippet(colors, &translation.text, text, colors.fg);
                job.wrap.max_width = ui.available_width();
                ui.label(job);
                if let Some(name) = translation.name.as_deref().filter(|n| !n.is_empty()) {
                    ui.label(RichText::new(format!("— {name}")).size(12.8).color(colors.muted));
                }
            }
        });
        go
    }
}

fn centred(ui: &mut Ui, colors: &Colors, text: &str) {
    ui.vertical_centered(|ui| ui.label(RichText::new(text).size(15.0).color(colors.muted)));
}

fn card(ui: &mut Ui, colors: &Colors, add: impl FnOnce(&mut Ui)) {
    egui::Frame::new()
        .fill(colors.card)
        .stroke(Stroke::new(1.0, colors.border))
        .corner_radius(10)
        .inner_margin(egui::Margin::symmetric(19, 16))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 8.0;
            add(ui);
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_result_shows_its_highlighted_text_when_it_has_one() {
        let mut result: SearchResult =
            serde_json::from_value(serde_json::json!({ "verse_key": "21:107", "text": "وَمَآ أَرْسَلْنَٰكَ" })).unwrap();
        assert_eq!(arabic_words(&result), [("وَمَآ".to_string(), false), ("أَرْسَلْنَٰكَ".to_string(), false)]);
        result.highlighted = Some("وَمَآ <em>أَرْسَلْنَٰكَ</em>".into());
        assert_eq!(arabic_words(&result).iter().map(|w| w.1).collect::<Vec<_>>(), [false, true]);
        result.highlighted = Some("  ".into());
        assert_eq!(arabic_words(&result).len(), 2, "an empty highlight falls back to the text");
        assert_ne!(word_key(1, 0), word_key(0, 1));
    }

    #[test]
    fn snippets_mark_their_matches() {
        let colors = Colors::of(&quran_core::settings::Settings::default(), false);
        let job = snippet(&colors, "a <em>mercy</em> to", 14.0, colors.fg);
        assert_eq!(job.text, "a mercy to");
        let marked: Vec<bool> = job.sections.iter().map(|s| s.format.background != Color32::TRANSPARENT).collect();
        assert_eq!(marked, [false, true, false]);
    }
}
