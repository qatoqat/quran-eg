//! The lists: all 114 surahs in a filterable grid, and the 30 ajzāʾ.

use std::collections::HashMap;
use std::sync::mpsc::Receiver;

use egui::{Color32, ColorImage, CornerRadius, FontId, Mesh, Rect, Sense, Shape, Stroke, StrokeKind, TextureHandle, TextureOptions, Ui, pos2, vec2};
use quran_core::api::{Chapter, Juz};
use quran_core::lists::{chapter_matches, find_chapter};
use quran_core::parse::{self, VerseSpan};
use quran_core::view::{Location, View};

use crate::glyphs::Palette;
use crate::kit;
use crate::reader::{COLUMN_PAD, Env};
use crate::sheet::{self, Request, Sprite, Text};
use crate::task::{self, Poll};

const GAP: f32 = 12.0;
const CARD_MIN: f32 = 290.0;
/// Size of the Arabic surah names, in points.
const NAME_SIZE: f32 = 22.0;

/// The Arabic names of the surahs, drawn once into one texture.
struct Names {
    texture: TextureHandle,
    sprites: HashMap<u64, Sprite>,
    pixels_per_point: f32,
}

type NamesJob = (f32, ColorImage, HashMap<u64, Sprite>);

#[derive(Default)]
pub struct Lists {
    filter: String,
    names: Option<Names>,
    names_job: Option<Receiver<NamesJob>>,
    names_failed: bool,
}

/// The content column, scrolling inside `ui`, as a child to lay widgets out in.
fn column(ui: &mut Ui, env: &Env<'_>, add: impl FnOnce(&mut Ui)) {
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        let width = (quran_core::settings::content_width_px(env.settings.content_width) as f32).min(ui.available_width());
        let side = ((ui.available_width() - width) / 2.0).max(0.0) + COLUMN_PAD;
        ui.horizontal_top(|ui| {
            ui.add_space(side);
            ui.vertical(|ui| {
                ui.set_width((width - 2.0 * COLUMN_PAD).max(120.0));
                ui.add_space(24.0);
                add(ui);
                ui.add_space(40.0);
            });
        });
    });
}

fn status(ui: &mut Ui, env: &Env<'_>, loading: bool, error: Option<&str>, what: &str) -> bool {
    let mut retry = false;
    if let Some(e) = error {
        kit::notice(ui, env.colors, kit::Notice::Error, |ui| ui.label(format!("Could not load the list of {what}: {e}")));
        ui.vertical_centered(|ui| retry = kit::pill(ui, env.colors, "Try again", kit::Pill::Primary).clicked());
    } else if loading {
        ui.vertical_centered(|ui| {
            ui.add_space(32.0);
            ui.label(egui::RichText::new("Loading…").size(16.0).color(env.colors.muted));
        });
    }
    retry
}

/// Cards in rows of equal columns; `card` paints one into its rectangle and
/// says how tall each one is.
fn grid(ui: &mut Ui, count: usize, height: impl Fn(usize) -> f32, mut card: impl FnMut(&mut Ui, usize, Rect)) {
    let width = ui.available_width();
    let columns = (((width + GAP) / (CARD_MIN + GAP)).floor() as usize).max(1);
    let card_width = (width - GAP * (columns - 1) as f32) / columns as f32;
    for row in 0..count.div_ceil(columns) {
        let items = row * columns..((row + 1) * columns).min(count);
        let tallest = items.clone().map(&height).fold(0.0, f32::max);
        let (rect, _) = ui.allocate_exact_size(vec2(width, tallest), Sense::hover());
        if ui.is_rect_visible(rect) {
            for (slot, i) in items.enumerate() {
                let left = rect.left() + slot as f32 * (card_width + GAP);
                card(ui, i, Rect::from_min_size(pos2(left, rect.top()), vec2(card_width, tallest)));
            }
        }
        ui.add_space(GAP);
    }
}

fn card_frame(ui: &Ui, env: &Env<'_>, rect: Rect, hot: bool) {
    let stroke = Stroke::new(1.0, if hot { env.colors.accent } else { env.colors.border });
    ui.painter().rect(rect, CornerRadius::same(10), env.colors.card, stroke, StrokeKind::Inside);
}

impl Lists {
    /// Free the names texture (the list is not on screen).
    pub fn release(&mut self) {
        self.names = None;
        self.names_job = None;
    }

    /// Draw the Arabic names once the text font and the chapter list are there.
    fn ensure_names(&mut self, ui: &Ui, env: &mut Env<'_>) {
        let ppp = ui.ctx().pixels_per_point();
        match self.names_job.as_ref().map(task::poll) {
            Some(Poll::Done((drawn_for, image, sprites))) => {
                self.names_job = None;
                let texture = ui.ctx().load_texture("surah-names", image, TextureOptions::LINEAR);
                self.names = Some(Names { texture, sprites, pixels_per_point: drawn_for });
            }
            Some(Poll::Lost) => {
                // The names could not be drawn: the list stays as it is, with the English names.
                self.names_job = None;
                self.names_failed = true;
            }
            Some(Poll::Pending) | None => {}
        }
        let current = self.names.as_ref().is_some_and(|n| n.pixels_per_point == ppp);
        if current || self.names_failed || self.names_job.is_some() || env.chapters.is_empty() {
            return;
        }
        let Some(fallback) = env.fonts.fallback() else { return };
        let requests = env
            .chapters
            .iter()
            .map(|c| Request { key: u64::from(c.id), glyph: None, text: Some(Text::plain(c.name_arabic.clone(), 1.0)) })
            .collect();
        self.names_job = Some(task::run(task::Pool::Draw, ui.ctx(), move || {
            let job = sheet::Job { requests, fonts: Vec::new(), fallback: Some(fallback), ornaments: None, px_per_em: NAME_SIZE * ppp, palette: Palette::Light };
            let sheet = sheet::build(job);
            (ppp, ColorImage::from_rgba_premultiplied([sheet.width, sheet.height], &sheet.rgba), sheet.sprites)
        }));
    }

    /// The surah list. Returns the view to open, if a card was clicked.
    pub fn surahs(&mut self, ui: &mut Ui, env: &mut Env<'_>, loading: bool, error: Option<&str>, retry: &mut bool) -> Option<Location> {
        self.ensure_names(ui, env);
        let mut open = None;
        column(ui, env, |ui| {
            ui.vertical_centered(|ui| {
                let hint = "Filter by number or name (e.g. 18, kahf, 2:255)";
                ui.add(egui::TextEdit::singleline(&mut self.filter).hint_text(hint).desired_width(400.0f32.min(ui.available_width() - 20.0)).margin(egui::Margin::symmetric(12, 8)));
            });
            ui.add_space(12.0);
            // A typed reference like "2:255" offers a direct jump.
            if let Ok(reference) = parse::parse_chapter_segment(&self.filter)
                && let Some(span) = reference.span
                && let Ok(view) = (View::Surah { chapter: reference.chapter, span: Some(span) }).checked()
            {
                let label = format!("Go to verse {}:{} →", reference.chapter, span.segment());
                if kit::link(ui, env.colors, &label, 15.0).clicked() {
                    open = Some(view.at(None));
                }
                ui.add_space(12.0);
            }
            *retry |= status(ui, env, loading, error, "surahs");
            let shown: Vec<&Chapter> = env.chapters.iter().filter(|c| chapter_matches(c, &self.filter)).collect();
            if shown.is_empty() && !env.chapters.is_empty() {
                ui.vertical_centered(|ui| {
                    let text = format!("No surah matches “{}”.", self.filter.trim());
                    ui.label(egui::RichText::new(text).color(env.colors.muted));
                });
            }
            let names = self.names.as_ref();
            grid(ui, shown.len(), |_| 62.0, |ui, i, rect| {
                let chapter = shown[i];
                let response = ui.interact(rect, ui.scope_id().with(("surah", chapter.id)), Sense::click());
                surah_card(ui, env, rect, chapter, names, response.hovered());
                if response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    open = Some(View::surah(chapter.id).at(None));
                }
            });
        });
        open
    }
}

/// number | name ... Arabic name, with the translated name and verse count on a second line.
fn surah_card(ui: &Ui, env: &Env<'_>, rect: Rect, chapter: &Chapter, names: Option<&Names>, hot: bool) {
    let colors = env.colors;
    card_frame(ui, env, rect, hot);
    let badge = Rect::from_min_size(rect.left_top() + vec2(16.0, 12.0), vec2(38.0, 38.0));
    ui.painter().rect_filled(badge, CornerRadius::same(8), colors.accent_tint());
    kit::centered_text(ui, badge.center(), &chapter.id.to_string(), 14.4, colors.accent);
    let mut right = rect.right() - 16.0;
    if let Some(names) = names
        && let Some(sprite) = names.sprites.get(&u64::from(chapter.id))
    {
        let mut mesh = Mesh::with_texture(names.texture.id());
        let baseline = rect.center().y + NAME_SIZE * 0.3;
        let mut pen = right;
        for part in &sprite.parts {
            pen -= part.advance * NAME_SIZE;
            let min = pos2(pen + part.left * NAME_SIZE, baseline - part.top * NAME_SIZE);
            let image = Rect::from_min_size(min, vec2(part.size[0], part.size[1]) * NAME_SIZE);
            mesh.add_rect_with_uv(image, Rect::from_min_max(pos2(part.uv[0], part.uv[1]), pos2(part.uv[2], part.uv[3])), colors.fg);
        }
        ui.painter().add(Shape::mesh(mesh));
        right -= sprite.advance * NAME_SIZE + 10.0;
    }
    let left = badge.right() + 12.8;
    let room = (right - left).max(40.0);
    let name = ui.painter().layout(chapter.name_simple.clone(), FontId::proportional(15.5), colors.fg, room);
    let meta = format!("{} · {} verses", chapter.translated_name.name, chapter.verses_count);
    let meta = ui.painter().layout(meta, FontId::proportional(12.8), colors.muted, room);
    let top = rect.center().y - (name.size().y + meta.size().y + 2.0) / 2.0;
    let meta_top = top + name.size().y + 2.0;
    ui.painter().galley(pos2(left, top), name, colors.fg);
    ui.painter().galley(pos2(left, meta_top), meta, colors.muted);
}

const JUZ_TITLE: f32 = 44.0;
const JUZ_ROW: f32 = 26.0;

/// The juz list: each juz with the verse ranges of its surahs.
pub fn juzs(ui: &mut Ui, env: &Env<'_>, loading: bool, error: Option<&str>, retry: &mut bool) -> Option<Location> {
    let mut open = None;
    column(ui, env, |ui| {
        *retry |= status(ui, env, loading, error, "juz");
        let ranges: Vec<Vec<(u16, VerseSpan)>> = env.juzs.iter().map(|j| parse::juz_mapping(&j.verse_mapping)).collect();
        let height = |i: usize| JUZ_TITLE + ranges[i].len() as f32 * JUZ_ROW + 10.0;
        grid(ui, env.juzs.len(), height, |ui, i, rect| {
            if let Some(to) = juz_card(ui, env, rect, &env.juzs[i], &ranges[i]) {
                open = Some(to);
            }
        });
    });
    open
}

fn juz_card(ui: &mut Ui, env: &Env<'_>, rect: Rect, juz: &Juz, ranges: &[(u16, VerseSpan)]) -> Option<Location> {
    let colors = env.colors;
    let mut open = None;
    card_frame(ui, env, rect, false);
    let inner = rect.shrink2(vec2(16.0, 0.0));
    let title = Rect::from_min_size(inner.left_top(), vec2(inner.width(), JUZ_TITLE));
    let response = ui.interact(title, ui.scope_id().with(("juz", juz.juz_number)), Sense::click());
    let ink = if response.hovered() { colors.accent } else { colors.fg };
    let text = ui.painter().layout_no_wrap(format!("Juz {}", juz.juz_number), FontId::proportional(17.0), ink);
    ui.painter().galley(pos2(title.left(), title.center().y - text.size().y / 2.0 + 2.0), text, ink);
    if response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
        open = Some(View::Juz(juz.juz_number).at(None));
    }
    for (n, (chapter, span)) in ranges.iter().enumerate() {
        let row = Rect::from_min_size(pos2(inner.left(), title.bottom() + n as f32 * JUZ_ROW), vec2(inner.width(), JUZ_ROW));
        let info = find_chapter(env.chapters, *chapter);
        let response = ui.interact(row, ui.scope_id().with(("juz-range", juz.juz_number, *chapter)), Sense::click());
        let ink = if response.hovered() { colors.accent } else { colors.fg };
        let name = info.map_or(format!("Surah {chapter}"), |c| c.name_simple.clone());
        let name = ui.painter().layout_no_wrap(format!("{chapter}. {name}"), FontId::proportional(15.0), ink);
        let verses = ui.painter().layout_no_wrap(format!("{}–{}", span.from, span.to), FontId::proportional(13.6), Color32::PLACEHOLDER);
        ui.painter().galley(pos2(row.left(), row.center().y - name.size().y / 2.0), name, ink);
        ui.painter().galley(pos2(row.right() - verses.size().x, row.center().y - verses.size().y / 2.0), verses, colors.muted);
        if response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            // The whole surah when the juz holds all of it, else the range.
            let whole = info.is_some_and(|c| span.from == 1 && span.to >= c.verses_count);
            open = Some(View::Surah { chapter: *chapter, span: (!whole).then_some(*span) }.at(None));
        }
    }
    open
}
