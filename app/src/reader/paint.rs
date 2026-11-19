//! Drawing a placed mushaf page: one mesh for all its sprites, text for the
//! meanings, and plain shapes for what has no sprite (yet).

use egui::{Align2, Color32, CornerRadius, FontId, Mesh, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, pos2, vec2};
use quran_core::api::Chapter;
use quran_core::glyph::MISSING_WORD;
use quran_core::lists::find_chapter;
use quran_core::mushaf_layout::{Item, PageBlock};
use quran_core::parse::revelation_label;

use super::blocks::{BANNER_SIDE_EM, Block, LoadedSheet, banner};
use super::place::{self, Metrics, Mode, PILL_HEIGHT, Placed, Row, RowKind, Word, label_key};
use crate::pointer::Lens;
use crate::sheet::{Kind, Sprite};
use crate::theme::Colors;

/// Where things sit inside the banner's frame, as shares of the frame's ink box
/// (measured for quran-dx from the QUL `header` glyph): the name cartouche in
/// the middle and the two roundels beside it.
mod art {
    /// Usable inside of the cartouche: `(x0, y0, x1, y1)`.
    pub const CARTOUCHE: (f32, f32, f32, f32) = (0.285, 0.17, 0.715, 0.83);
    /// Roundel centres (left, right) and their common centre line.
    pub const ROUNDEL_X: (f32, f32) = (0.206, 0.793);
    pub const ROUNDEL_Y: f32 = 0.496;
    /// Inner radius of a roundel, as a share of the frame's height.
    pub const ROUNDEL_R: f32 = 0.283;
}

/// Size of a sprite's (single) image, in em.
fn ink(sprite: &Sprite) -> Option<egui::Vec2> {
    let part = sprite.parts.first()?;
    (part.size[0] > 0.0 && part.size[1] > 0.0).then(|| vec2(part.size[0], part.size[1]))
}

/// Add the part of a quad that lies in the lens to `mesh`, cut into small cells
/// whose corners carry the lens's strength there — which gives the lens its
/// round, soft-edged shape without a mask.
fn light(mesh: &mut Mesh, lens: &Lens, rect: Rect, uv: Rect) {
    const CELL: f32 = 3.0;
    let area = rect.intersect(lens.bounds());
    if !area.is_positive() {
        return;
    }
    let cells = |length: f32| (length / CELL).ceil().max(1.0) as usize;
    let (columns, rows) = (cells(area.width()), cells(area.height()));
    let corner = |column: usize, row: usize| {
        let at = pos2(
            area.left() + area.width() * column as f32 / columns as f32,
            area.top() + area.height() * row as f32 / rows as f32,
        );
        let share = (at - rect.min) / rect.size();
        let texel = uv.min + share * uv.size();
        let level = (lens.strength(at) * 255.0).round() as u8;
        (at, texel, level)
    };
    for row in 0..rows {
        for column in 0..columns {
            let corners = [corner(column, row), corner(column + 1, row), corner(column, row + 1), corner(column + 1, row + 1)];
            if corners.iter().all(|(_, _, level)| *level == 0) {
                continue;
            }
            let base = mesh.vertices.len() as u32;
            for (pos, uv, level) in corners {
                let color = Color32::from_rgba_premultiplied(level, level, level, level);
                mesh.vertices.push(egui::epaint::Vertex { pos, uv, color });
            }
            mesh.add_triangle(base, base + 1, base + 2);
            mesh.add_triangle(base + 2, base + 1, base + 3);
        }
    }
}

/// Ascent and descent (em) assumed until a page font says otherwise.
const DEFAULT_METRICS: (f32, f32) = (1.0, 0.5);

/// What went wrong drawing a page, if anything: said on the page itself.
fn alert(block: &Block, placed: &Placed) -> Option<String> {
    if placed.word_count != block.cells {
        return Some(format!("Only {} of this page's {} words could be laid out.", placed.word_count, block.cells));
    }
    let failed = block.sheet.as_ref().map_or(0, |s| s.failed_glyphs.len());
    let (s, verb) = if failed == 1 { ("", "is") } else { ("s", "are") };
    (failed > 0).then(|| {
        format!("{failed} word{s} on this page could not be drawn with the page font and {verb} shown as plain text, marked “!”.")
    })
}

/// A hovered word's box and what its tooltip says (each line, and whether it is in italics).
pub type Tip = (Rect, Vec<(String, bool)>);

pub struct Canvas<'a> {
    pub painter: &'a Painter,
    pub colors: &'a Colors,
    pub metrics: &'a Metrics,
    pub chapters: &'a [Chapter],
    pub pixels_per_point: f32,
    /// What is on screen: rows outside it are skipped.
    pub clip: Rect,
    /// Where the mouse is, if over the pages.
    pub hover: Option<Pos2>,
    /// The hovered word and what it says (word-by-word "on hover"), found while painting.
    pub tip: std::cell::RefCell<Option<Tip>>,
    /// The reading pointer's lens, if it glows.
    pub lens: Option<Lens>,
    /// The page's quads inside the lens, drawn from the vivid sheet.
    pub vivid: std::cell::RefCell<Option<Mesh>>,
}

impl Canvas<'_> {
    fn snap(&self, v: f32) -> f32 {
        (v * self.pixels_per_point).round() / self.pixels_per_point
    }

    /// Paint `block` with the left edge of its lines and its top at `origin`.
    /// Returns the rectangle of its page number pill.
    pub fn page(&self, origin: Pos2, block: &Block, placed: &Placed) -> Rect {
        let m = self.metrics;
        let sheet = block.sheet.as_ref();
        let mut mesh = sheet.map(|s| Mesh::with_texture(s.texture.id()));
        *self.vivid.borrow_mut() = sheet.and_then(|s| s.vivid.as_ref()).map(|t| Mesh::with_texture(t.id()));
        // Glyphs reach beyond their line box: a row is skipped only when well off screen.
        let reach = m.em;
        for row in &placed.rows {
            let top = origin.y + row.y;
            if top + row.height + reach < self.clip.top() || top - reach > self.clip.bottom() {
                continue;
            }
            self.row(pos2(origin.x, top), row, &block.page, sheet, mesh.as_mut());
        }
        if let Some(mesh) = mesh.filter(|m| !m.is_empty()) {
            self.painter.add(Shape::mesh(mesh));
        }
        // Over them, inside the pointer's lens: the same glyphs, more vivid.
        if let Some(mesh) = self.vivid.borrow_mut().take().filter(|m| !m.is_empty()) {
            self.painter.add(Shape::mesh(mesh));
        }
        if let Some(alert) = alert(block, placed) {
            // Above the page's first line, in the UI font.
            let at = pos2(origin.x + m.line_width / 2.0, origin.y + 14.0);
            self.painter.text(at, Align2::CENTER_CENTER, alert, FontId::proportional(12.8), self.colors.danger);
        }
        self.pill(origin, block, placed)
    }

    fn row(&self, at: Pos2, row: &Row, page: &PageBlock, sheet: Option<&LoadedSheet>, mut mesh: Option<&mut Mesh>) {
        let m = self.metrics;
        match &row.kind {
            RowKind::Banner { chapter } => self.banner(at, *chapter, sheet, mesh),
            RowKind::Bismillah { words, meaning } => {
                for word in words {
                    self.word(at, word, page, sheet, mesh.as_deref_mut());
                }
                let y = at.y + m.line_height() + m.meaning_gap.max(0.0);
                let meaning = self.painter.layout_job(place::bismillah_job(meaning, m));
                self.painter.galley(pos2(at.x + m.line_width / 2.0, y), meaning, m.muted);
            }
            RowKind::Line { words } => {
                for word in words {
                    self.word(at, word, page, sheet, mesh.as_deref_mut());
                }
            }
            RowKind::Flow { note, words } => {
                // A verse that could not be placed on its mushaf lines: a dashed box with a note.
                let frame = Rect::from_min_size(at, vec2(m.line_width, row.height - m.line_gap));
                let stroke = Stroke::new(1.0, self.colors.warn_border);
                let corners = [frame.left_top(), frame.right_top(), frame.right_bottom(), frame.left_bottom(), frame.left_top()];
                self.painter.extend(Shape::dashed_line(&corners, stroke, 5.0, 4.0));
                self.painter.galley(at + vec2(8.0, 6.0), self.painter.layout_job(place::note_job(note, m)), m.muted);
                for word in words {
                    self.word(at, word, page, sheet, mesh.as_deref_mut());
                }
            }
        }
    }

    /// One word: its sprite (or what stands in for it) centred in its box on
    /// the line's baseline, and its meaning underneath.
    fn word(&self, row: Pos2, word: &Word, page: &PageBlock, sheet: Option<&LoadedSheet>, mesh: Option<&mut Mesh>) {
        let (m, c) = (self.metrics, self.colors);
        let left = row.x + word.x;
        let top = row.y + word.y;
        let centre = left + word.width / 2.0;
        let (ascent, descent) = sheet.and_then(|s| s.metrics).unwrap_or(DEFAULT_METRICS);
        let baseline = top + (m.line_height() - (ascent + descent) * m.em) / 2.0 + ascent * m.em;
        let middle = top + m.line_height() / 2.0;
        let sprite = sheet.and_then(|s| s.sprites.get(&word.key));
        match (word.mode, sprite, mesh) {
            (Mode::Glyph, Some(sprite), Some(mesh)) => {
                self.sprite(mesh, sprite, centre + sprite.advance * m.em / 2.0, baseline, c.fg);
            }
            (Mode::Text { ringed }, Some(sprite), Some(mesh)) => {
                let ink = if ringed { c.accent } else { c.fg };
                if ringed {
                    // An ayah number without its medallion: centred in a ring.
                    let part = sprite.parts.first().copied();
                    let height = part.map_or(0.0, |p| p.size[1] * m.em);
                    let top = part.map_or(0.0, |p| p.top * m.em);
                    self.painter.circle_stroke(pos2(centre, middle), m.em * 0.42, Stroke::new(1.0, ink));
                    self.sprite(mesh, sprite, centre + sprite.advance * m.em / 2.0, middle - height / 2.0 + top, ink);
                } else {
                    self.sprite(mesh, sprite, centre + sprite.advance * m.em / 2.0, baseline, ink);
                }
                // A word whose glyph the page font should have drawn but could not: marked.
                if sheet.is_some_and(|s| s.failed_glyphs.contains(&word.key)) {
                    self.problem_mark(pos2(left, top));
                }
            }
            (Mode::Missing, ..) => {
                let font = FontId::proportional(m.em * 0.45);
                self.painter.text(pos2(centre, middle), Align2::CENTER_CENTER, MISSING_WORD, font, c.muted);
            }
            // No font yet (or the sprite is on its way): a marked box where the word will be.
            _ => {
                let width = (word.width.min(m.em * 2.4) - 4.0).max(m.em * 0.3);
                let slot = Rect::from_center_size(pos2(centre, middle + m.em * 0.05), vec2(width, m.em * 0.7));
                self.painter.rect(slot, CornerRadius::same(4), Colors::faint(c.muted, 0.12), Stroke::new(1.0, Colors::faint(c.muted, 0.45)), StrokeKind::Inside);
            }
        }
        let cell = word.cell.filter(|_| word.meaning.is_some()).and_then(|(item, index)| match page.items.get(item)? {
            Item::Line { cells, .. } | Item::Flow { cells, .. } => cells.get(index),
            _ => None,
        });
        // "On hover": the hovered word is tinted and says what it means in a tooltip.
        let slot = Rect::from_min_size(pos2(left, top), vec2(word.width, m.line_height()));
        if m.tooltip
            && self.hover.is_some_and(|p| slot.contains(p))
            && let Some((item, index)) = word.cell
            && let Some(Item::Line { cells, .. } | Item::Flow { cells, .. }) = page.items.get(item)
            && let Some(cell) = cells.get(index).filter(|c| !c.is_end)
        {
            let (transliteration, translation) = place::word_notes(cell, m);
            let lines: Vec<(String, bool)> =
                transliteration.map(|t| (t, true)).into_iter().chain(translation.map(|t| (t, false))).collect();
            if !lines.is_empty() {
                self.painter.rect_filled(slot.expand(2.0), CornerRadius::same(4), Colors::faint(c.accent, 0.14));
                *self.tip.borrow_mut() = Some((slot, lines));
            }
        }
        if let Some(job) = cell.and_then(|c| place::meaning_job(c, m)) {
            let meaning = self.painter.layout_job(job);
            self.painter.galley(pos2(centre, top + m.line_height() + m.meaning_gap), meaning, m.muted);
        }
    }

    /// Show the hovered word's tooltip, above everything.
    pub fn finish(&self, ctx: &egui::Context) {
        if let Some((slot, lines)) = self.tip.borrow_mut().take() {
            let lines: Vec<(&str, bool)> = lines.iter().map(|(text, italic)| (text.as_str(), *italic)).collect();
            // Above the word, or under it on the first line of the window.
            let above = slot.top() - self.clip.top() > 60.0;
            crate::kit::tip(ctx, self.colors, slot, &lines, !above);
        }
    }

    /// Add a sprite's parts to the mesh, right to left from `right`, on `baseline`.
    fn sprite(&self, mesh: &mut Mesh, sprite: &Sprite, right: f32, baseline: f32, ink: Color32) {
        let em = self.metrics.em;
        let tint = match sprite.kind {
            Kind::Mask => ink,
            Kind::Color => Color32::WHITE,
        };
        let mut pen = right;
        for part in &sprite.parts {
            pen -= part.advance * em;
            if part.size[0] <= 0.0 || part.size[1] <= 0.0 {
                continue;
            }
            let min = pos2(self.snap(pen + part.left * em), self.snap(baseline - part.top * em));
            let rect = Rect::from_min_size(min, vec2(part.size[0] * em, part.size[1] * em));
            let uv = Rect::from_min_max(pos2(part.uv[0], part.uv[1]), pos2(part.uv[2], part.uv[3]));
            mesh.add_rect_with_uv(rect, uv, tint);
            if sprite.kind == Kind::Color
                && let Some(lens) = &self.lens
                && let Some(vivid) = self.vivid.borrow_mut().as_mut()
            {
                light(vivid, lens, rect, uv);
            }
        }
    }

    /// A red badge in the UI font, so it can't be hidden by the problem it reports.
    fn problem_mark(&self, at: Pos2) {
        let centre = at + vec2(2.0, 8.0);
        self.painter.circle_filled(centre, 8.0, Color32::from_rgb(0xc6, 0x28, 0x28));
        self.painter.text(centre, Align2::CENTER_CENTER, "!", FontId::proportional(12.0), Color32::WHITE);
    }

    /// The surah banner, one mushaf line tall, as in the printed mushaf: an
    /// ornamental frame with the calligraphic name in its cartouche, the place of
    /// revelation in the right roundel and the number of verses in the left one.
    /// Until the ornament fonts are there (or if they can't be had): a framed line
    /// with the same content.
    fn banner(&self, at: Pos2, chapter: u16, sheet: Option<&LoadedSheet>, mesh: Option<&mut Mesh>) {
        let (m, c) = (self.metrics, self.colors);
        let row = Rect::from_min_size(at, vec2(m.line_width, m.line_height()));
        let sprite = |chapter: u16, part: usize| sheet.and_then(|s| s.sprites.get(&label_key("h", chapter, part)));
        let label = |part: usize| sprite(chapter, part);
        let Some(mesh) = mesh else { return self.banner_words(row, chapter) };

        if let (Some(frame), Some(name)) = (sprite(0, banner::FRAME), label(banner::ORNAMENT_NAME))
            && let (Some(frame_ink), Some(name_ink)) = (ink(frame), ink(name))
        {
            // The frame: one line tall, or as wide as the line if that is less.
            let scale = (row.height() / frame_ink.y).min(row.width() / frame_ink.x);
            let frame_rect = Rect::from_center_size(row.center(), frame_ink * scale);
            self.stretch(mesh, frame, frame_rect, c.ornament);
            // The name, fitted into the cartouche.
            let (x0, y0, x1, y1) = art::CARTOUCHE;
            let room = vec2((x1 - x0) * frame_rect.width(), (y1 - y0) * frame_rect.height());
            let fit = (room.x / name_ink.x).min(room.y / name_ink.y);
            let centre = frame_rect.min + vec2((x0 + x1) / 2.0 * frame_rect.width(), (y0 + y1) / 2.0 * frame_rect.height());
            self.stretch(mesh, name, Rect::from_center_size(centre, name_ink * fit), c.fg);
            // The roundels: the place on the right; "آياتها" over the count on the left.
            let radius = art::ROUNDEL_R * frame_rect.height();
            let middle = frame_rect.top() + art::ROUNDEL_Y * frame_rect.height();
            let x = |share: f32| frame_rect.left() + share * frame_rect.width();
            let mut put = |part: usize, centre: Pos2, size: f32| {
                if let Some((sprite, ink)) = label(part).and_then(|s| Some((s, ink(s)?))) {
                    // (`ink` is in em of the page; the label was drawn at BANNER_SIDE_EM of it.)
                    let scale = size / BANNER_SIDE_EM;
                    self.stretch(mesh, sprite, Rect::from_center_size(centre, ink * scale), c.fg);
                }
            };
            put(banner::PLACE, pos2(x(art::ROUNDEL_X.1), middle), radius * 0.66);
            put(banner::VERSES, pos2(x(art::ROUNDEL_X.0), middle - radius * 0.38), radius * 0.44);
            put(banner::COUNT, pos2(x(art::ROUNDEL_X.0), middle + radius * 0.34), radius * 0.66);
            return;
        }

        let Some(name) = label(banner::NAME) else { return self.banner_words(row, chapter) };
        self.banner_frame(row);
        let pad = m.em * 0.7;
        // Centre each label's ink in the frame's height.
        let mut draw = |sprite: &Sprite, right: f32| {
            let part = sprite.parts.first().copied();
            let (height, top) = part.map_or((0.0, 0.0), |p| (p.size[1] * m.em, p.top * m.em));
            self.sprite(mesh, sprite, right, row.center().y - height / 2.0 + top, c.fg);
        };
        draw(name, row.center().x + name.advance * m.em / 2.0);
        if let Some(place) = label(banner::PLACE) {
            draw(place, row.right() - pad);
        }
        // "آياتها" then the count, reading right to left from the label.
        if let (Some(verses), Some(count)) = (label(banner::VERSES), label(banner::COUNT)) {
            let gap = m.em * 0.1;
            let left = row.left() + pad;
            draw(count, left + count.advance * m.em);
            draw(verses, left + (count.advance + verses.advance) * m.em + gap);
        }
    }

    fn banner_frame(&self, row: Rect) {
        let stroke = Stroke::new(1.0, self.colors.ornament);
        self.painter.rect_stroke(row, CornerRadius::same(6), stroke, StrokeKind::Inside);
        self.painter.rect_stroke(row.shrink(2.5), CornerRadius::same(4), stroke, StrokeKind::Inside);
    }

    /// The banner before any text font is there: the same content in the UI font.
    fn banner_words(&self, row: Rect, chapter: u16) {
        self.banner_frame(row);
        let text = match find_chapter(self.chapters, chapter) {
            Some(info) => {
                format!("Surah {} · {} · {} verses", info.name_simple, revelation_label(&info.revelation_place), info.verses_count)
            }
            None => format!("Surah {chapter}"),
        };
        let font = FontId::proportional(self.metrics.em * 0.36);
        self.painter.text(row.center(), Align2::CENTER_CENTER, text, font, self.colors.fg);
    }

    /// Draw a sprite's (single) image into `rect`.
    fn stretch(&self, mesh: &mut Mesh, sprite: &Sprite, rect: Rect, ink: Color32) {
        let Some(part) = sprite.parts.first() else { return };
        let tint = if sprite.kind == Kind::Mask { ink } else { Color32::WHITE };
        let uv = Rect::from_min_max(pos2(part.uv[0], part.uv[1]), pos2(part.uv[2], part.uv[3]));
        let rect = Rect::from_min_size(pos2(self.snap(rect.left()), self.snap(rect.top())), rect.size());
        mesh.add_rect_with_uv(rect, uv, tint);
    }

    /// The page number under the page, and the rule that ends the page.
    fn pill(&self, origin: Pos2, block: &Block, placed: &Placed) -> Rect {
        let (m, c) = (self.metrics, self.colors);
        let centre = pos2(origin.x + m.line_width / 2.0, origin.y + placed.pill_y + PILL_HEIGHT / 2.0);
        let mut rect = Rect::from_center_size(centre, vec2(40.0, PILL_HEIGHT));
        if let Some(page) = block.page.page {
            let text = self.painter.layout_no_wrap(page.to_string(), FontId::proportional(13.6), c.muted);
            rect = Rect::from_center_size(centre, vec2((text.size().x + 20.0).max(38.0), PILL_HEIGHT));
            if rect.intersects(self.clip) {
                self.painter.rect_stroke(rect, CornerRadius::same(255), Stroke::new(1.0, c.border), StrokeKind::Inside);
                self.painter.galley(centre - text.size() / 2.0, text, c.muted);
            }
        }
        let rule = origin.y + placed.pill_y + PILL_HEIGHT + 12.0;
        self.painter.hline(origin.x..=origin.x + m.line_width, rule, Stroke::new(1.0, c.border));
        rect
    }
}
