//! The app's own small widgets, painted in the theme's colours.

use std::sync::Arc;

use egui::{Align2, Color32, CornerRadius, CursorIcon, FontId, Galley, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2};

use crate::theme::Colors;

pub fn galley(ui: &Ui, text: &str, size: f32, color: Color32, wrap: f32) -> Arc<Galley> {
    ui.painter().layout(text.to_owned(), FontId::proportional(size), color, wrap)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pill {
    /// Outlined, on the card colour (navigation).
    Outline,
    /// Filled with the accent (the main action).
    Primary,
    /// A segment of a toggle: filled with the accent when `on`.
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

const ARROW: f32 = 20.0;

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
            Pill::Outline => (14.4, vec2(15.0, 6.5)),
            Pill::Primary => (15.0, vec2(26.0, 9.0)),
            Pill::Segment { .. } => (14.4, vec2(13.0, 3.5)),
            Pill::Small => (12.8, vec2(10.0, 3.0)),
            Pill::Tag => (13.6, vec2(9.0, 2.5)),
        }
    }
}

/// A rounded button sized to its text.
pub fn pill(ui: &mut Ui, colors: &Colors, text: &str, style: Pill) -> Response {
    arrow_pill(ui, colors, text, style, Arrow::None)
}

pub fn arrow_pill(ui: &mut Ui, colors: &Colors, text: &str, style: Pill, arrow: Arrow) -> Response {
    let (size, pad) = style.metrics();
    let extra = if arrow == Arrow::None { 0.0 } else { ARROW };
    let text = galley(ui, text, size, Color32::PLACEHOLDER, f32::INFINITY);
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
            let stroke = Stroke::new(1.4, ink);
            ui.painter().line_segment([tip, tip + vec2(back * 12.0, 0.0)], stroke);
            ui.painter().line_segment([tip, tip + vec2(back * 4.5, -4.5)], stroke);
            ui.painter().line_segment([tip, tip + vec2(back * 4.5, 4.5)], stroke);
        }
    }
    response.on_hover_cursor(CursorIcon::PointingHand)
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Search,
    Settings,
    Close,
}

/// A round icon button; `on` while what it opens is open.
pub fn icon_button(ui: &mut Ui, colors: &Colors, icon: Icon, on: bool) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(35.0), Sense::click());
    let hot = on || response.hovered() || response.has_focus();
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        if hot {
            painter.circle_filled(rect.center(), rect.width() / 2.0, colors.accent_tint());
        }
        let stroke = Stroke::new(1.8, if hot { colors.accent } else { colors.muted });
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
        }
    }
    response.on_hover_cursor(CursorIcon::PointingHand)
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
