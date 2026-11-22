//! The app's own small widgets, painted in the theme's colours.

use std::sync::Arc;

use egui::{Align, Align2, Color32, CornerRadius, CursorIcon, FontId, Galley, Layout, Painter, Pos2, Rect, Response, Sense, Shape, Stroke, StrokeKind, Ui, UiBuilder, Vec2, pos2, vec2};

use crate::theme::Colors;

pub fn galley(ui: &Ui, text: &str, size: f32, color: Color32, wrap: f32) -> Arc<Galley> {
    layout(ui.painter(), text, size, color, wrap)
}

fn layout(painter: &Painter, text: &str, size: f32, color: Color32, wrap: f32) -> Arc<Galley> {
    painter.layout(text.to_owned(), FontId::proportional(size), color, wrap)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pill {
    /// Outlined, on the card colour (navigation).
    Outline,
    /// Filled with the accent (the main action).
    Primary,
    /// A segment of a toggle: filled with the accent when `on` (in a capsule: `segments`).
    Segment { on: bool },
    /// A small outlined action (copy, clear).
    Small,
    /// A label on an accent tint (a verse key, a badge).
    Tag,
}

/// An arrow drawn beside a pill's text, pointing the way the pill leads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arrow {
    None,
    /// Before the text, pointing left.
    Left,
    /// After the text, pointing right.
    Right,
}

const ARROW: f32 = 24.0;

/// Size of a pill with this text.
pub fn pill_size(ui: &Ui, text: &str, style: Pill, arrow: Arrow) -> Vec2 {
    let (size, pad) = style.metrics();
    let extra = if arrow == Arrow::None { 0.0 } else { ARROW };
    galley(ui, text, size, Color32::PLACEHOLDER, f32::INFINITY).size() + 2.0 * pad + vec2(extra, 0.0)
}

impl Pill {
    /// Font size and padding.
    fn metrics(self) -> (f32, Vec2) {
        match self {
            Pill::Outline => (16.0, vec2(20.0, 10.0)),
            Pill::Primary => (15.0, vec2(26.0, 9.0)),
            Pill::Segment { .. } => (14.4, vec2(13.0, 3.5)),
            Pill::Small => (13.6, vec2(12.0, 4.0)),
            Pill::Tag => (13.6, vec2(10.0, 4.0)),
        }
    }
}

/// A rounded button sized to its text.
pub fn pill(ui: &mut Ui, colors: &Colors, text: &str, style: Pill) -> Response {
    arrow_pill(ui, colors, text, style, Arrow::None)
}

pub fn arrow_pill(ui: &mut Ui, colors: &Colors, text: &str, style: Pill, arrow: Arrow) -> Response {
    let (size, pad) = style.metrics();
    let text = galley(ui, text, size, Color32::PLACEHOLDER, f32::INFINITY);
    padded_pill(ui, colors, text, pad, style, arrow)
}

/// A pill of `text` (laid out in `Color32::PLACEHOLDER`, painted in the style's ink) padded by `pad`.
fn padded_pill(ui: &mut Ui, colors: &Colors, text: Arc<Galley>, pad: Vec2, style: Pill, arrow: Arrow) -> Response {
    let extra = if arrow == Arrow::None { 0.0 } else { ARROW };
    let (rect, response) = ui.allocate_exact_size(text.size() + 2.0 * pad + vec2(extra, 0.0), Sense::click());
    let hot = response.hovered() || response.has_focus();
    let (fill, stroke, ink) = match style {
        Pill::Outline if hot => (colors.card, colors.accent, colors.accent),
        Pill::Outline => (colors.card, colors.border, colors.fg),
        Pill::Primary => (colors.accent, colors.accent, colors.on_accent),
        Pill::Segment { on: true } => (colors.accent, colors.accent, colors.on_accent),
        Pill::Segment { on: false } if hot => (Color32::TRANSPARENT, Color32::TRANSPARENT, colors.fg),
        Pill::Segment { on: false } => (Color32::TRANSPARENT, Color32::TRANSPARENT, colors.muted),
        Pill::Small if hot => (Color32::TRANSPARENT, colors.accent, colors.accent),
        Pill::Small => (Color32::TRANSPARENT, colors.border, colors.muted),
        Pill::Tag if hot => (Colors::faint(colors.accent, 0.22), Color32::TRANSPARENT, colors.fg),
        Pill::Tag => (colors.accent_tint(), Color32::TRANSPARENT, colors.fg),
    };
    if ui.is_rect_visible(rect) {
        let radius = CornerRadius::same(255);
        ui.painter().rect(rect, radius, fill, Stroke::new(1.0, stroke), StrokeKind::Inside);
        let shift = match arrow {
            Arrow::None => 0.0,
            Arrow::Left => extra / 2.0,
            Arrow::Right => -extra / 2.0,
        };
        ui.painter().galley(rect.center() - text.size() / 2.0 + vec2(shift, 0.0), text, ink);
        // The arrow: a shaft and a head, in the text's colour.
        let (tip, back) = match arrow {
            Arrow::None => (None, 0.0),
            Arrow::Left => (Some(pos2(rect.left() + pad.x, rect.center().y)), 1.0),
            Arrow::Right => (Some(pos2(rect.right() - pad.x, rect.center().y)), -1.0),
        };
        if let Some(tip) = tip {
            let stroke = Stroke::new(1.6, ink);
            ui.painter().line_segment([tip, tip + vec2(back * 14.0, 0.0)], stroke);
            ui.painter().line_segment([tip, tip + vec2(back * 5.0, -5.0)], stroke);
            ui.painter().line_segment([tip, tip + vec2(back * 5.0, 5.0)], stroke);
        }
    }
    response.on_hover_cursor(CursorIcon::PointingHand)
}

/// The capsule's margin around its segments.
const INSET: f32 = 3.0;
/// The least padding either side of a segment's label, where a row is short of room.
const TIGHT: f32 = 7.0;

/// A capsule of exclusive choices, as the header's Surah | Juz | Page: segments on the page's colour in a thin border,
/// the chosen one filled with the accent. Laid out once, to be sized and placed before it is shown.
pub struct Capsule {
    texts: Vec<Arc<Galley>>,
    pad: Vec2,
    inset: f32,
    size: Vec2,
}

/// A capsule shown: its rect and its segments' buttons.
pub struct Segments {
    pub rect: Rect,
    pub buttons: Vec<Response>,
}

impl Segments {
    pub fn hovered(&self) -> Option<usize> {
        self.buttons.iter().position(|b| b.hovered() || b.has_focus())
    }

    pub fn clicked(&self) -> Option<usize> {
        self.buttons.iter().position(Response::clicked)
    }
}

impl Capsule {
    /// `labels` at `scale` times the header's size, their padding narrowed (down to `TIGHT`) to fit `width`.
    pub fn new(painter: &Painter, labels: &[&str], scale: f32, width: f32) -> Capsule {
        let (size, pad) = Pill::Segment { on: false }.metrics();
        let texts: Vec<_> = labels.iter().map(|l| layout(painter, l, size * scale, Color32::PLACEHOLDER, f32::INFINITY)).collect();
        let words: f32 = texts.iter().map(|t| t.size().x).sum();
        let sides = 2.0 * labels.len().max(1) as f32;
        let inset = INSET * scale;
        let fit = (width - 2.0 * inset - words) / sides;
        let pad = vec2((pad.x * scale).min(fit).max(TIGHT * scale), pad.y * scale);
        let high = texts.iter().map(|t| t.size().y).fold(0.0, f32::max);
        let size = vec2(words + sides * pad.x, high + 2.0 * pad.y) + Vec2::splat(2.0 * inset);
        Capsule { texts, pad, inset, size }
    }

    /// (Only the toasts, which the web build has not, size one before showing it.)
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub fn size(&self) -> Vec2 {
        self.size
    }

    /// Shows the capsule where `ui` puts the next widget; segment `on` is the chosen one.
    pub fn show(&self, ui: &mut Ui, colors: &Colors, on: Option<usize>) -> Segments {
        let (rect, _) = ui.allocate_exact_size(self.size, Sense::hover());
        ui.painter().rect(rect, CornerRadius::same(255), colors.bg, Stroke::new(1.0, colors.border), StrokeKind::Inside);
        let inner = rect.shrink(self.inset);
        let buttons = ui
            .scope_builder(UiBuilder::new().max_rect(inner).layout(Layout::left_to_right(Align::Center)), |ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                let pill = |(i, text): (usize, &Arc<Galley>)| padded_pill(ui, colors, text.clone(), self.pad, Pill::Segment { on: on == Some(i) }, Arrow::None);
                self.texts.iter().enumerate().map(pill).collect()
            })
            .inner;
        Segments { rect, buttons }
    }
}

/// A capsule of `labels` at the header's size, in the width `ui` has.
pub fn segments(ui: &mut Ui, colors: &Colors, labels: &[&str], on: Option<usize>) -> Segments {
    Capsule::new(ui.painter(), labels, 1.0, ui.available_width()).show(ui, colors, on)
}

/// Text that acts as a link.
pub fn link(ui: &mut Ui, colors: &Colors, text: &str, size: f32) -> Response {
    let text = galley(ui, text, size, Color32::PLACEHOLDER, f32::INFINITY);
    let (rect, response) = ui.allocate_exact_size(text.size(), Sense::click());
    if ui.is_rect_visible(rect) {
        if response.hovered() || response.has_focus() {
            ui.painter().hline(rect.x_range(), rect.bottom() - 1.0, Stroke::new(1.0, colors.accent));
        }
        ui.painter().galley(rect.min, text, colors.accent);
    }
    response.on_hover_cursor(CursorIcon::PointingHand)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Icon {
    Search,
    Settings,
    Close,
    /// Recording the recitation: a ring, filled red while recording.
    Record,
    /// Following the recitation with the voice pointer: a microphone.
    Listen(Mic),
    /// Memorizing: three lines of text, the middle one ending at an ayah's mark; while on, the words are dashes and the
    /// mark stays (as the mode draws the page).
    Hifz,
    /// Replaying the last recording with the voice pointer.
    Replay(Play),
}

/// The Replay button: a triangle to play; a spinner on the rim while its listener starts; a square to stop.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Play {
    Idle,
    /// The spinner this far (0..1) through its turn.
    Starting(f32),
    Playing,
}

/// The Listen button's microphone.
#[derive(Debug, Clone, Copy, PartialEq)]
// (The web build has no listener: its microphone would only ever be off.)
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub enum Mic {
    Off,
    /// Joining or starting the listener: a spinner on the rim, this far (0..1) through its turn.
    Connecting(f32),
    /// Listening: the disc filled from the bottom to the input's `level` (0..=1), turned as far as
    /// `loud` (0..=1) to the danger colour.
    On { level: f32, loud: f32 },
    /// No audio arriving, or a capture error: the microphone struck through.
    Trouble,
}

/// A round icon button; `on` while what it opens is open.
pub fn icon_button(ui: &mut Ui, colors: &Colors, icon: Icon, on: bool) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(35.0), Sense::click());
    let hot = on || response.hovered() || response.has_focus();
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        // A microphone that hears turns the danger colour as it gets too loud; one in trouble is dead
        // (grey) on the danger colour's tint.
        let ink = match icon {
            Icon::Listen(Mic::On { loud, .. }) => colors.accent.lerp_to_gamma(colors.danger, loud),
            Icon::Listen(Mic::Trouble) => colors.muted,
            _ if hot => colors.accent,
            _ => colors.muted,
        };
        if hot {
            let tint = if icon == Icon::Listen(Mic::Trouble) { colors.danger } else { ink };
            painter.circle_filled(rect.center(), rect.width() / 2.0, Colors::faint(tint, 0.12));
        }
        let stroke = Stroke::new(1.8, ink);
        let c = rect.center();
        match icon {
            Icon::Search => {
                painter.circle_stroke(c + vec2(-1.2, -1.2), 5.8, stroke);
                painter.line_segment([c + vec2(3.2, 3.2), c + vec2(7.2, 7.2)], stroke);
            }
            Icon::Settings => {
                // A cog: a hub, a rim and eight teeth.
                painter.circle_stroke(c, 2.6, stroke);
                painter.circle_stroke(c, 6.0, stroke);
                for i in 0..8 {
                    let dir = Vec2::angled(i as f32 * std::f32::consts::TAU / 8.0);
                    painter.line_segment([c + dir * 6.4, c + dir * 8.8], Stroke::new(2.6, stroke.color));
                }
            }
            Icon::Close => {
                painter.line_segment([c + vec2(-5.0, -5.0), c + vec2(5.0, 5.0)], stroke);
                painter.line_segment([c + vec2(5.0, -5.0), c + vec2(-5.0, 5.0)], stroke);
            }
            Icon::Record => {
                if on {
                    painter.circle_filled(c, 6.0, egui::Color32::from_rgb(220, 38, 38));
                } else {
                    painter.circle_stroke(c, 6.0, stroke);
                    painter.circle_filled(c, 2.4, stroke.color);
                }
            }
            Icon::Listen(mic) => microphone(painter, colors, c, rect.width() / 2.0, mic, stroke),
            Icon::Hifz => {
                // Each line from the right to where it ends (the middle one at the mark, the last one short).
                for (y, left) in [(-5.0, -8.0), (0.0, -3.6), (5.0, -2.0)] {
                    if on {
                        let mut x = 8.0;
                        while x - 2.4 >= left - 0.01 {
                            painter.line_segment([c + vec2(x, y), c + vec2(x - 2.4, y)], stroke);
                            x -= 4.0;
                        }
                    } else {
                        painter.line_segment([c + vec2(left, y), c + vec2(8.0, y)], stroke);
                    }
                }
                painter.circle_stroke(c + vec2(-6.4, 0.0), 2.0, Stroke::new(1.4, ink));
            }
            Icon::Replay(Play::Playing) => {
                painter.rect_filled(Rect::from_center_size(c, Vec2::splat(9.0)), CornerRadius::same(2), ink);
            }
            Icon::Replay(play) => {
                painter.add(Shape::closed_line(vec![c + vec2(-3.5, -5.5), c + vec2(5.5, 0.0), c + vec2(-3.5, 5.5)], stroke));
                if let Play::Starting(turn) = play {
                    spinner(painter, colors, c, rect.width() / 2.0, turn);
                }
            }
        }
    }
    response.on_hover_cursor(CursorIcon::PointingHand)
}

/// A quarter of the rim of a disc (centre `c`, radius `r`), just inside it, `turn` (0..1) through its turn clockwise.
fn spinner(painter: &Painter, colors: &Colors, c: Pos2, r: f32, turn: f32) {
    let start = std::f32::consts::TAU * turn;
    let arc = (0..=12).map(|k| c + (r - 1.1) * Vec2::angled(start + std::f32::consts::FRAC_PI_2 * k as f32 / 12.0)).collect();
    painter.add(Shape::line(arc, Stroke::new(2.2, colors.accent)));
}

/// A capsule on a stand, with the cradle around its lower half: filled while listening, over the
/// liquid of its level; with a spinner on the rim while connecting; struck through in trouble.
fn microphone(painter: &Painter, colors: &Colors, c: Pos2, r: f32, mic: Mic, stroke: Stroke) {
    if let Mic::On { level, .. } = mic
        && let Some(liquid) = liquid(c, r, level, Colors::faint(stroke.color, if colors.dark { 0.34 } else { 0.26 }))
    {
        painter.add(liquid);
    }
    let capsule = Rect::from_center_size(c - vec2(0.0, 2.5), vec2(6.0, 10.0));
    if matches!(mic, Mic::On { .. }) {
        painter.rect_filled(capsule, CornerRadius::same(3), stroke.color);
    } else {
        painter.rect_stroke(capsule, CornerRadius::same(3), stroke, StrokeKind::Middle);
    }
    let cradle = (0..=12)
        .map(|k| {
            let a = std::f32::consts::PI * k as f32 / 12.0;
            c + vec2(-6.0 * a.cos(), -1.0 + 6.0 * a.sin())
        })
        .collect();
    painter.add(Shape::line(cradle, stroke));
    painter.line_segment([c + vec2(0.0, 5.0), c + vec2(0.0, 8.0)], stroke);
    match mic {
        Mic::Connecting(turn) => spinner(painter, colors, c, r, turn),
        Mic::Trouble => {
            // Cut out of the glyph under it (in the disc's colour on the header), then drawn.
            let ends = [c + vec2(-7.0, -8.0), c + vec2(7.0, 8.0)];
            painter.line_segment(ends, Stroke::new(4.6, colors.card.lerp_to_gamma(colors.danger, 0.12)));
            painter.line_segment(ends, Stroke::new(1.8, colors.danger));
        }
        Mic::Off | Mic::On { .. } => {}
    }
}

/// The liquid in a disc (centre `c`, radius `r`) filled to `level` (0..=1) from the bottom: the
/// segment under its surface (a convex polygon, clockwise from the surface's right end), the whole
/// disc, or nothing.
fn liquid(c: Pos2, r: f32, level: f32, fill: Color32) -> Option<Shape> {
    if level < 0.01 {
        return None;
    }
    if level > 0.995 {
        return Some(Shape::circle_filled(c, r, fill));
    }
    // The surface's ends are `theta` either side of the bottom; a point every 1/8 radian or less.
    let theta = (1.0 - 2.0 * level).acos();
    let k = 2 + (8.0 * theta) as usize;
    let points = (0..=k)
        .map(|i| {
            let phi = theta - 2.0 * theta * i as f32 / k as f32;
            c + r * vec2(phi.sin(), phi.cos())
        })
        .collect();
    Some(Shape::convex_polygon(points, fill, Stroke::NONE))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Notice {
    Warning,
    Error,
}

/// A framed notice across the available width; `add` fills it.
pub fn notice<R>(ui: &mut Ui, colors: &Colors, kind: Notice, add: impl FnOnce(&mut Ui) -> R) -> R {
    let (fill, border, ink) = match kind {
        Notice::Warning => (colors.warn_bg, colors.warn_border, colors.warn_fg),
        Notice::Error => (colors.error_bg, colors.error_border, colors.error_fg),
    };
    egui::Frame::new()
        .fill(fill)
        .stroke(Stroke::new(1.0, border))
        .corner_radius(8)
        .inner_margin(egui::Margin::symmetric(14, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.visuals_mut().override_text_color = Some(ink);
            add(ui)
        })
        .inner
}

/// Text centred at `center`, in the UI font.
pub fn centered_text(ui: &Ui, center: egui::Pos2, text: &str, size: f32, color: Color32) -> Rect {
    ui.painter().text(center, Align2::CENTER_CENTER, text, FontId::proportional(size), color)
}

/// A thin vertical scroll bar at the right edge of `area`. Returns the new
/// offset while it is being dragged.
pub fn scroll_bar(ui: &mut Ui, colors: &Colors, area: Rect, offset: f32, content: f32) -> Option<f32> {
    let view = area.height();
    if content <= view + 1.0 {
        return None;
    }
    let track = Rect::from_min_max(pos2(area.right() - 10.0, area.top()), area.max);
    let thumb_height = (view * view / content).clamp(32.0, view);
    let travel = view - thumb_height;
    let max_offset = content - view;
    let top = track.top() + travel * (offset / max_offset).clamp(0.0, 1.0);
    let thumb = Rect::from_min_size(pos2(track.left() + 2.0, top), vec2(6.0, thumb_height));
    let response = ui.interact(track, ui.scope_id().with("scroll-bar"), Sense::click_and_drag());
    let hot = response.hovered() || response.dragged();
    let color = if hot { colors.muted } else { Colors::faint(colors.muted, 0.45) };
    ui.painter().rect_filled(thumb, CornerRadius::same(3), color);
    let pointer = response.interact_pointer_pos()?;
    if response.dragged() || response.clicked() {
        // The thumb's middle follows the pointer.
        let t = ((pointer.y - track.top() - thumb_height / 2.0) / travel.max(1.0)).clamp(0.0, 1.0);
        return Some(t * max_offset);
    }
    None
}

/// A tooltip, dark in every theme, centred on `anchor`: under it if `below`,
/// else above it. Each line may be in italics.
pub fn tip(ctx: &egui::Context, colors: &Colors, anchor: Rect, lines: &[(&str, bool)], below: bool) {
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("tip")));
    let mut job = egui::text::LayoutJob { halign: egui::Align::Center, ..Default::default() };
    job.wrap.max_width = 256.0;
    for (i, (text, italics)) in lines.iter().enumerate() {
        let format = egui::TextFormat { italics: *italics, ..egui::TextFormat::simple(FontId::proportional(13.6), crate::theme::TIP_FG) };
        job.append(if i > 0 { "\n" } else { "" }, 0.0, format.clone());
        job.append(text, 0.0, format);
    }
    let text = painter.layout_job(job);
    let size = text.size() + vec2(20.0, 11.0);
    let screen = ctx.content_rect().shrink(4.0);
    let centre = anchor.center().x.clamp(screen.left() + size.x / 2.0, (screen.right() - size.x / 2.0).max(screen.left() + size.x / 2.0));
    let top = if below { anchor.bottom() + 8.0 } else { anchor.top() - 8.0 - size.y };
    let rect = Rect::from_min_size(pos2(centre - size.x / 2.0, top), size);
    painter.rect(rect, CornerRadius::same(6), colors.tip_bg, Stroke::new(1.0, colors.tip_border), StrokeKind::Inside);
    // A centred galley is placed by its middle.
    painter.galley(pos2(centre, rect.top() + 5.5), text, crate::theme::TIP_FG);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_liquid_is_the_segment_of_the_disc_under_its_level() {
        let (c, r) = (pos2(100.0, 50.0), 17.5);
        assert!(liquid(c, r, 0.009, Color32::RED).is_none(), "nothing below 1%");
        assert!(matches!(liquid(c, r, 0.996, Color32::RED), Some(Shape::Circle(_))), "the whole disc near full");
        for level in [0.01, 0.125, 0.5, 0.75, 0.995] {
            let Some(Shape::Path(segment)) = liquid(c, r, level, Color32::RED) else { panic!("a segment at {level}") };
            let points = &segment.points;
            assert!(points.len() <= 28 && segment.closed);
            assert!(points.iter().all(|p| (p.distance(c) - r).abs() < 1e-3), "every point on the circle");
            let surface = c.y + r - 2.0 * r * level;
            assert!([points[0], points[points.len() - 1]].iter().all(|p| (p.y - surface).abs() < 1e-3));
            assert!(points.iter().all(|p| p.y >= surface - 1e-3), "nothing above the surface");
        }
        let Some(Shape::Path(half)) = liquid(c, r, 0.5, Color32::RED) else { panic!() };
        let ends = (half.points[0], half.points[half.points.len() - 1]);
        assert!(ends.0.distance(pos2(c.x + r, c.y)) < 1e-3 && ends.1.distance(pos2(c.x - r, c.y)) < 1e-3);
    }
}
