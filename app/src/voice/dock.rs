//! The pieces of the preview's dock: the passage drawn as the reader draws a page (the page-1 font's glyphs,
//! else the Hafs face's text, else placeholder boxes) and laid out on the canvas; in a lane under each line the
//! reader's caret and the listener's ticks; the bar over the canvas and the scenes' timeline under it.

use std::collections::HashMap;
use std::sync::Arc;

use egui::{Align, Align2, Color32, ColorImage, CursorIcon, FontId, Layout, Mesh, Painter, Pos2, Rangef, Rect, Response, RichText, Sense, Shape, Stroke, TextureHandle, TextureOptions, Ui, UiBuilder, pos2, vec2};
use quran_core::fonts::FontSet;
use quran_core::settings::{self, MUSHAF_LINE, Settings};

use super::link::Heard;
use super::marker::point;
use super::script::{Cadence, FIRST, Glyph, LINES, Scene};
use crate::arabic::ArabicFont;
use crate::fonts::FontStore;
use crate::glyphs::{PageGlyphs, Palette};
use crate::kit::{self, Pill};
use crate::reader::{LOOSE_GAP, NUMBER_EM, RINGED_EM, Slot, UTHMANI_EM, baseline, placeholder, quads, ring};
use crate::sheet::{self, Kind, Request, Sprite, TextFont};
use crate::theme::Colors;

/// Under each line of the passage, the lane of the marks; between a lane and the next line, a little space (em).
const LANE: f32 = 0.34;
const LINE_SPACE: f32 = 0.08;

/// What the passage's sheet was drawn from.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Spec {
    set: FontSet,
    palette: Palette,
    px_per_em: f32,
    page: bool,
    fallback: bool,
}

/// The passage's glyphs in one texture, as a page's sheet.
pub struct Text {
    spec: Spec,
    texture: TextureHandle,
    sprites: HashMap<u64, Sprite>,
    metrics: Option<(f32, f32)>,
}

/// The passage's glyphs, ayah ends included, in reading order.
fn glyphs() -> impl Iterator<Item = &'static Glyph> {
    LINES.iter().flat_map(|line| line.iter())
}

impl Text {
    /// The passage's sheet at `em` points a glyph em: kept while it is what is wanted; drawn again (at once: its
    /// glyphs take a millisecond) for another font, palette or size, or once a font has arrived; none while
    /// there is nothing to draw with.
    pub fn ensure(text: &mut Option<Text>, ctx: &egui::Context, settings: &Settings, colors: &Colors, fonts: &mut FontStore, em: f32) {
        let set = FontSet::of(settings.mushaf_font);
        let (page, fallback) = (fonts.page(set, 1), fonts.fallback());
        let spec = Spec {
            set,
            // A single-colour font has no palettes: the theme tints it when drawn.
            palette: if set.coloured() { colors.glyphs } else { Palette::Light },
            px_per_em: (em * ctx.pixels_per_point()).ceil(),
            page: page.is_some(),
            fallback: fallback.is_some(),
        };
        if text.as_ref().is_none_or(|t| t.spec != spec) {
            *text = (spec.page || spec.fallback).then(|| Text::draw(ctx, spec, page, fallback));
        }
    }

    fn draw(ctx: &egui::Context, spec: Spec, page: Option<Arc<dyn PageGlyphs>>, fallback: Option<Arc<ArabicFont>>) -> Text {
        let requests = glyphs()
            .enumerate()
            .map(|(i, g)| {
                let end = g.letters == 0;
                let (em, font) = if end { (NUMBER_EM, TextFont::Number) } else { (UTHMANI_EM, TextFont::Fallback) };
                Request { key: i as u64, glyph: Some((0, g.code.to_string())), text: Some(sheet::Text { text: g.text.into(), em, font }) }
            })
            .collect();
        let job = sheet::Job { requests, fonts: vec![page], fallback, ornaments: None, px_per_em: spec.px_per_em, palette: spec.palette };
        let sheet = sheet::build(job);
        let image = ColorImage::from_rgba_premultiplied([sheet.width, sheet.height], &sheet.rgba);
        let texture = ctx.load_texture("voice-preview", image, TextureOptions::LINEAR);
        Text { spec, texture, sprites: sheet.sprites, metrics: sheet.metrics }
    }

    fn sprite(&self, i: usize) -> Option<&Sprite> {
        self.sprites.get(&(i as u64))
    }
}

/// Glyph `i`'s width in em: its sprite's, or before there is one its measured width. An ayah's number drawn as
/// text has a ring around it.
fn advance(text: Option<&Text>, i: usize, g: &Glyph) -> f32 {
    match text.and_then(|t| t.sprite(i)) {
        Some(s) if !s.glyph && g.letters == 0 => s.advance.max(RINGED_EM),
        Some(s) => s.advance,
        None => g.advance,
    }
}

/// The passage on the canvas: each glyph's box (as tall as its line) and baseline, the words' slots for the
/// marker, the text size, and each line's lane (its words' span, and the lane's top and height).
pub struct Placed {
    pub em: f32,
    pub slots: Vec<Slot>,
    boxes: Vec<(Rect, f32)>,
    lanes: Vec<(Rangef, f32, f32)>,
}

/// Lays the passage out in `area` as a page lays out a centred line, each line under the one before (its lane
/// between them) and centred, at the page's text size `page_em` or smaller to fit both ways, the words as far
/// apart as the page's (`spacing`).
pub fn place(area: Rect, page_em: f32, spacing: u8, text: Option<&Text>) -> Placed {
    let gap = settings::word_gap(spacing) + LOOSE_GAP;
    let all: Vec<&Glyph> = glyphs().collect();
    let advances: Vec<f32> = all.iter().enumerate().map(|(i, g)| advance(text, i, g)).collect();
    // Each line's glyphs (indices into the passage) and its width in em.
    let mut first = 0;
    let lines: Vec<(std::ops::Range<usize>, f32)> = LINES
        .iter()
        .map(|line| {
            let range = first..first + line.len();
            first = range.end;
            let width = advances[range.clone()].iter().sum::<f32>() + gap * (line.len() - 1) as f32;
            (range, width)
        })
        .collect();
    let widest = lines.iter().map(|l| l.1).fold(0.0, f32::max);
    let tall = 3.0 * (MUSHAF_LINE + LANE) + 2.0 * LINE_SPACE;
    let em = page_em.min(area.width() / widest).min(area.height() / tall);
    let (height, lane, space) = (em * MUSHAF_LINE, em * LANE, em * LINE_SPACE);
    let mut top = area.center().y - tall * em / 2.0;
    let mut placed = Placed { em, slots: Vec::new(), boxes: Vec::new(), lanes: Vec::new() };
    let base = |top: f32| baseline(top, em, text.and_then(|t| t.metrics));
    for (range, width) in lines {
        let mut right = area.center().x + width * em / 2.0;
        placed.lanes.push((Rangef::new(right - width * em, right), top + height, lane));
        for i in range {
            let rect = Rect::from_min_max(pos2(right - advances[i] * em, top), pos2(right, top + height));
            placed.boxes.push((rect, base(top)));
            if all[i].letters > 0 {
                placed.slots.push((all[i].loc, rect, base(top), em));
            }
            right -= (advances[i] + gap) * em;
        }
        top += height + lane + space;
    }
    placed
}

/// Draws the passage: glyphs from the sheet, text in the Hafs face (an ayah's number in a ring, as the reader
/// draws one without its medallion), placeholders where nothing can be drawn yet.
pub fn draw_text(painter: &Painter, placed: &Placed, text: Option<&Text>, colors: &Colors) {
    let ppp = painter.ctx().pixels_per_point();
    let snap = |v: f32| (v * ppp).round() / ppp;
    let em = placed.em;
    let mut mesh = text.map(|t| Mesh::with_texture(t.texture.id()));
    for ((i, g), &(rect, base)) in glyphs().enumerate().zip(&placed.boxes) {
        let (Some(sprite), Some(mesh)) = (text.and_then(|t| t.sprite(i)), mesh.as_mut()) else {
            placeholder(painter, colors, rect.center(), rect.width(), em);
            continue;
        };
        let ringed = !sprite.glyph && g.letters == 0;
        let ink = if ringed { colors.accent } else { colors.fg };
        let baseline = if ringed { ring(painter, rect.center(), sprite, em, ink) } else { base };
        let tint = if sprite.kind == Kind::Mask { ink } else { Color32::WHITE };
        for (quad, uv) in quads(sprite, (rect.center().x + sprite.advance * em / 2.0, baseline), em, snap) {
            mesh.add_rect_with_uv(quad, uv, tint);
        }
    }
    if let Some(mesh) = mesh.filter(|m| !m.is_empty()) {
        painter.add(Shape::mesh(mesh));
    }
}

/// The point of word position `c` (index in the whole Quran plus the share read) on the passage, and its word's
/// box, as the marker finds it.
fn spot(placed: &Placed, c: f32) -> Option<(f32, Rect)> {
    let anchor = Heard { loc: placed.slots.first()?.0, word: FIRST, letter: 0, letters: 1, state: String::new(), conf: 0.0, speed: 0.0, t: 0.0, slid: None, alts: Vec::new(), plan: None };
    let (at, i) = point(c, &anchor, &placed.slots, 0.0)?;
    Some((at.x, placed.slots[i].1))
}

/// The caret: a triangle `size` tall pointing up at `tip`.
fn caret(painter: &Painter, tip: Pos2, size: f32, color: Color32) {
    painter.add(Shape::convex_polygon(vec![tip, tip + vec2(size * 0.6, size), tip + vec2(-size * 0.6, size)], color, Stroke::NONE));
}

/// A tick: a bar `size` tall from `top`.
fn tick(painter: &Painter, top: Pos2, size: f32, color: Color32) {
    painter.rect_filled(Rect::from_min_max(top - vec2(1.0, 0.0), top + vec2(1.0, size)), 0.0, color);
}

/// In the lane under each line, along a faint track under its words: a tick for each word position the listener
/// sent lately (`ticks`: position and how faded, 0..=1), in the accent, and a caret where the reader really is
/// (`truth`, words into the passage).
pub fn draw_marks(painter: &Painter, placed: &Placed, truth: Option<f32>, ticks: impl Iterator<Item = (f32, f32)>, colors: &Colors) {
    for &(span, top, lane) in &placed.lanes {
        painter.hline(span, top + lane * 0.5, Stroke::new(1.0, Colors::faint(colors.muted, 0.25)));
    }
    let lane = |word: Rect| placed.lanes.iter().find(|(_, top, _)| (top - word.bottom()).abs() < 0.5).map(|&(_, top, lane)| (top, lane));
    for (c, faded) in ticks {
        if let Some((x, (top, lane))) = spot(placed, c).and_then(|(x, word)| Some((x, lane(word)?))) {
            tick(painter, pos2(x, top + lane * 0.12), lane * 0.76, Colors::faint(colors.accent, 1.0 - faded));
        }
    }
    if let Some((x, (top, lane))) = truth.and_then(|p| spot(placed, FIRST as f32 + p)).and_then(|(x, word)| Some((x, lane(word)?))) {
        caret(painter, pos2(x, top + lane * 0.08), lane * 0.84, colors.fg);
    }
}

/// What the marks under the lines are, on the canvas's frame at its top right (as a fieldset's legend): "▲ you
/// ▍heard".
pub fn legend(painter: &Painter, canvas: Rect, colors: &Colors) {
    let font = FontId::proportional(10.5);
    let (heard, you) = (painter.layout_no_wrap("heard".into(), font.clone(), colors.muted), painter.layout_no_wrap("you".into(), font, colors.muted));
    let (mark, gap) = (7.0, 4.0);
    let width = mark + gap + you.size().x + 10.0 + mark + gap + heard.size().x;
    let y = canvas.top();
    let card = Rect::from_min_max(pos2(canvas.right() - 14.0 - width - 5.0, y - 6.0), pos2(canvas.right() - 14.0 + 5.0, y + 6.0));
    painter.rect_filled(card, 4.0, colors.bg);
    let mut x = card.left() + 5.0;
    caret(painter, pos2(x + mark / 2.0, y - mark / 2.0), mark, colors.fg);
    x += mark + gap;
    painter.galley(pos2(x, y - you.size().y / 2.0), you.clone(), colors.muted);
    x += you.size().x + 10.0;
    tick(painter, pos2(x + mark / 2.0, y - mark / 2.0), mark, colors.accent);
    x += mark + gap;
    painter.galley(pos2(x, y - heard.size().y / 2.0), heard, colors.muted);
}

/// A line of muted text across `rect`'s foot, cut short with an ellipsis where it would not fit.
pub fn subtitle(painter: &Painter, rect: Rect, text: &str, colors: &Colors) {
    let mut job = egui::text::LayoutJob::single_section(text.to_owned(), egui::TextFormat::simple(FontId::proportional(11.5), colors.muted));
    job.wrap = egui::text::TextWrapping { max_width: rect.width(), max_rows: 1, break_anywhere: false, overflow_character: Some('…') };
    let galley = painter.layout_job(job);
    painter.galley(pos2(rect.left(), rect.bottom() - galley.size().y), galley, colors.muted);
}

/// What the reader asked of the dock.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Action {
    Cadence(Cadence),
    Play,
    Pause,
    Fold(bool),
    Jump(Scene),
}

/// The cadence's capsule, smaller than the header's in the bar's row.
const CAPSULE: f32 = 0.8;
const EVEN: &str = "A perfect listener: every step alike, on every tick. A stutter here is the curves' and times' doing.";
const HEARD: &str = "As quran-voice hears a reader (measured on your recordings): a word held, then most of one at once, a step back at a breath.";

/// The bar over the canvas: the preview's `scale` (folded: none, and the whole bar unfolds it), the listener,
/// play or pause, and folding it away.
pub fn bar(ui: &mut Ui, rect: Rect, colors: &Colors, scale: Option<f32>, cadence: Cadence, playing: bool) -> Option<Action> {
    let mut action = None;
    let folded = scale.is_none();
    if folded && ui.interact(rect, egui::Id::unique("voice-preview-bar"), Sense::click()).on_hover_cursor(CursorIcon::PointingHand).clicked() {
        action = Some(Action::Fold(false));
    }
    let mut ui = ui.new_child(UiBuilder::new().id_salt("voice-preview-bar").max_rect(rect).layout(Layout::left_to_right(Align::Center)));
    ui.spacing_mut().item_spacing = vec2(4.0, 0.0);
    match scale {
        None => drop(ui.label(RichText::new("PREVIEW").size(12.0).color(colors.muted))),
        Some(scale) => {
            let label = ui.label(RichText::new(format!("PREVIEW · {:.0}%", scale * 100.0)).size(12.0).color(colors.muted));
            label.on_hover_text(format!(
                "The preview's text is {:.0}% of the page's. On the page the pointer goes {:.1} times as far in the same time, so \
                 a stutter shows more there. Under each line the caret is where you are and the ticks where the listener heard \
                 you: they show the rhythm at any size. A scene plays again and again until another is chosen.",
                scale * 100.0,
                1.0 / scale
            ));
            ui.add_space(6.0);
            let options = [(Cadence::Even, "Even", EVEN), (Cadence::Heard, "As heard", HEARD)];
            let capsule = kit::Capsule::new(ui.painter(), &options.map(|o| o.1), CAPSULE, f32::INFINITY);
            let choice = capsule.show(&mut ui, colors, options.iter().position(|o| o.0 == cadence));
            if let Some(i) = choice.clicked() {
                action = Some(Action::Cadence(options[i].0));
            }
            for (button, option) in choice.buttons.into_iter().zip(options) {
                button.on_hover_text(option.2);
            }
        }
    }
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        if chevron(ui, colors, folded).clicked() {
            action = Some(Action::Fold(!folded));
        }
        let (name, asked) = if playing { ("Pause", Action::Pause) } else { ("Play", Action::Play) };
        if !folded && kit::pill(ui, colors, name, Pill::Small).clicked() {
            action = Some(asked);
        }
    });
    action
}

/// A chevron that folds the preview away (pointing down) or brings it back (pointing right).
fn chevron(ui: &mut Ui, colors: &Colors, folded: bool) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::click());
    let ink = if response.hovered() { colors.accent } else { colors.muted };
    let c = rect.center();
    let points = if folded { [vec2(-2.5, -5.0), vec2(2.5, 0.0), vec2(-2.5, 5.0)] } else { [vec2(-5.0, -2.5), vec2(0.0, 2.5), vec2(5.0, -2.5)] };
    ui.painter().add(Shape::line(points.iter().map(|p| c + *p).collect(), Stroke::new(1.6, ink)));
    response.on_hover_cursor(CursorIcon::PointingHand).on_hover_text(if folded { "Show the preview" } else { "Fold the preview away" })
}

/// The scenes in a row, the one playing (`playing`) tinted with a playhead `share` of the way through it. Clicking
/// a scene asks to play it.
pub fn timeline(ui: &mut Ui, rect: Rect, colors: &Colors, playing: Scene, share: f64) -> Option<Action> {
    let painter = ui.painter_at(rect);
    let width = rect.width() / Scene::ALL.len() as f32;
    let mut action = None;
    for (k, scene) in Scene::ALL.into_iter().enumerate() {
        let cell = Rect::from_min_size(pos2(rect.left() + width * k as f32, rect.top()), vec2(width, rect.height())).shrink2(vec2(1.0, 0.0));
        let response = ui.interact(cell, egui::Id::unique(("voice-preview-scene", k)), Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
        let on = scene == playing;
        let fill = match (on, response.hovered()) {
            (true, _) => colors.accent_tint(),
            (false, true) => Colors::faint(colors.muted, 0.1),
            (false, false) => Color32::TRANSPARENT,
        };
        painter.rect_filled(cell, 4.0, fill);
        painter.text(cell.center(), Align2::CENTER_CENTER, scene.label(), FontId::proportional(10.5), if on { colors.fg } else { colors.muted });
        if on {
            let x = cell.left() + share.clamp(0.0, 1.0) as f32 * cell.width();
            painter.vline(x, cell.y_range(), Stroke::new(2.0, colors.accent));
        }
        if response.on_hover_text(scene.hint()).clicked() {
            action = Some(Action::Jump(scene));
        }
    }
    action
}

/// Where the canvas meets the dock: its frame on the page's own background, so the pointer's opacity and blur
/// are seen on the ground they will have.
pub fn frame(painter: &Painter, canvas: Rect, colors: &Colors) {
    painter.rect(canvas, 8.0, colors.bg, Stroke::new(1.0, colors.border), egui::StrokeKind::Inside);
}

/// The card shown while the page turns to where the reader went on.
pub fn turning(painter: &Painter, canvas: Rect, colors: &Colors) {
    painter.rect_filled(canvas.shrink(1.0), 8.0, Colors::faint(colors.bg, 0.8));
    painter.text(canvas.center(), Align2::CENTER_CENTER, "the page turns to where you read", FontId::proportional(13.0), colors.fg);
}

/// The rule over the dock, as under the panel's header.
pub fn rule(painter: &Painter, dock: Rect, colors: &Colors) {
    painter.hline(dock.x_range(), dock.top(), Stroke::new(1.0, colors.border));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The canvas's words' area in the dock.
    fn area(width: f32, height: f32) -> Rect {
        Rect::from_min_size(pos2(100.0, 300.0), vec2(width, height))
    }

    #[test]
    fn the_passage_fits_its_area_both_ways() {
        // Wide and low (as the Hafs face's text, narrower than the page font's glyphs): the height decides.
        for (width, height, page_em) in [(300.0, 128.0, 50.0), (2000.0, 128.0, 50.0), (300.0, 2000.0, 50.0), (2000.0, 2000.0, 20.0)] {
            let placed = place(area(width, height), page_em, 2, None);
            let inside = area(width, height).expand(0.5);
            assert!(placed.boxes.iter().all(|(r, _)| inside.contains_rect(*r)), "{width}x{height}: {:?}", placed.boxes);
            assert!(placed.lanes.iter().all(|(span, top, lane)| inside.contains(pos2(span.min, top + lane)) && inside.contains(pos2(span.max, *top))));
            assert!(placed.em <= page_em + 1e-4);
        }
        assert_eq!(place(area(2000.0, 2000.0), 20.0, 2, None).em, 20.0, "no larger than the page's");
    }

    #[test]
    fn each_line_has_a_lane_of_its_own_under_its_words() {
        let placed = place(area(300.0, 128.0), 50.0, 2, None);
        assert_eq!(placed.lanes.len(), 3);
        for (k, (span, top, lane)) in placed.lanes.iter().enumerate() {
            let words: Vec<&Rect> = placed.boxes.iter().map(|(r, _)| r).filter(|r| (r.bottom() - top).abs() < 0.5).collect();
            assert!(!words.is_empty() && *lane > 5.0, "line {k}");
            assert!(words.iter().all(|r| r.left() >= span.min - 0.01 && r.right() <= span.max + 0.01), "line {k}");
            // Clear of the next line.
            if let Some((_, next, _)) = placed.lanes.get(k + 1) {
                assert!(top + lane < next - placed.em * MUSHAF_LINE);
            }
        }
    }
}
