//! The offer to save the Quran for reading offline, and the saving's progress, in a toast like the voice pointer's:
//! the header's look (its card, border and type) on one line, the buttons at its end in the header's capsule. It
//! sits a little above the foot of the reading area, centred, and stays there.

use egui::{Color32, CornerRadius, FontId, Id, LayerId, Order, Rect, Sense, Stroke, StrokeKind, UiBuilder, Vec2, pos2, vec2};

use super::job::Kind;
use crate::kit::Capsule;
use crate::theme::Colors;

/// What the toast says.
#[derive(Debug, Clone, PartialEq)]
pub enum Note {
    /// The first run's offer.
    Offer,
    /// A job under way, and how far it has come ("37 of 114 surahs"; none while it counts).
    Saving(Kind, Option<String>),
    /// A job done, for a moment.
    Saved(Kind),
    /// A job that stopped short, and why.
    Failed(Kind, String),
}

/// A button clicked on the toast.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Click {
    Download,
    NotNow,
    Stop,
    Retry,
    Dismiss,
}

/// The toast's line, and its buttons with what each does.
pub fn text(note: &Note) -> (String, &'static [(&'static str, Click)]) {
    const STOP: &[(&str, Click)] = &[("Cancel", Click::Stop)];
    const RETRY: &[(&str, Click)] = &[("Try again", Click::Retry), ("Dismiss", Click::Dismiss)];
    match note {
        Note::Offer => ("Save the Quran on this computer for reading offline?".into(), &[("Download", Click::Download), ("Not now", Click::NotNow)]),
        Note::Saving(kind, count) => {
            let doing = if *kind == Kind::Download { "Saving for offline use" } else { "Updating the offline copy" };
            (count.as_ref().map_or_else(|| format!("{doing}…"), |count| format!("{doing} · {count}")), STOP)
        }
        Note::Saved(Kind::Download) => ("Saved for offline use".into(), &[]),
        Note::Saved(Kind::Update) => ("Offline copy updated".into(), &[]),
        Note::Failed(Kind::Download, why) => (format!("Saving for offline use stopped: {why}"), RETRY),
        Note::Failed(Kind::Update, why) => (format!("Updating the offline copy stopped: {why}"), RETRY),
    }
}

/// The type: the header's name (its title is 16.8).
const TEXT: f32 = 16.0;
/// The padding: with the capsule's height, a one-line toast is as tall as the header.
const PAD: Vec2 = vec2(16.0, 10.0);
/// Between the line and the buttons.
const GAP: f32 = 16.0;
const RADIUS: f32 = 12.0;
/// Between the toast and the foot of the reading area.
const FOOT: f32 = 24.0;

/// Shows the toast near the foot of `area`; returns the button clicked.
pub fn show(ctx: &egui::Context, note: &Note, area: Rect, colors: &Colors) -> Option<Click> {
    let (line, clicks) = text(note);
    let painter = ctx.layer_painter(LayerId::new(Order::Foreground, Id::unique("offline-toast-layout")));
    let labels: Vec<&str> = clicks.iter().map(|c| c.0).collect();
    let buttons = (!labels.is_empty()).then(|| Capsule::new(&painter, &labels, 1.0, f32::INFINITY));
    let room = buttons.as_ref().map_or(Vec2::ZERO, |b| b.size() + vec2(GAP, 0.0));
    let wrap = (area.width() - 24.0 - 2.0 * PAD.x - room.x).max(60.0);
    let line = painter.layout(line, FontId::proportional(TEXT), colors.fg, wrap);
    // Without buttons it is as tall as with them: the toast changes its words, not its size.
    let high = Capsule::new(&painter, &[""], 1.0, f32::INFINITY).size().y;
    let size = vec2(line.size().x + room.x, line.size().y.max(high)) + 2.0 * PAD;
    let at = pos2((area.center().x - size.x / 2.0).max(area.left()), (area.bottom() - FOOT - size.y).max(area.top()));
    let shown = egui::Area::new(Id::unique("offline-toast")).order(Order::Foreground).fixed_pos(at).show(ctx, |ui| {
        let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
        let radius = CornerRadius::same(RADIUS as u8);
        let shadow = egui::Shadow { offset: [0, 3], blur: 14, spread: 0, color: Color32::from_black_alpha(40) };
        ui.painter().add(shadow.as_shape(rect, radius));
        ui.painter().rect(rect, radius, colors.card, Stroke::new(1.0, colors.border), StrokeKind::Inside);
        let inner = rect.shrink2(PAD);
        ui.painter().galley(pos2(inner.left(), inner.center().y - line.size().y / 2.0), line, colors.fg);
        let buttons = buttons?;
        let room = buttons.size();
        let place = Rect::from_min_size(pos2(inner.right() - room.x, inner.center().y - room.y / 2.0), room);
        ui.scope_builder(UiBuilder::new().max_rect(place), |ui| buttons.show(ui, colors, None).clicked()).inner
    });
    shown.inner.map(|i| clicks[i].1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_toast_offers_then_counts_surahs() {
        let (line, buttons) = text(&Note::Offer);
        assert_eq!(line, "Save the Quran on this computer for reading offline?");
        assert_eq!(buttons, &[("Download", Click::Download), ("Not now", Click::NotNow)]);
        let saving = |count: &str| Note::Saving(Kind::Download, Some(count.into()));
        assert_eq!(text(&saving("37 of 114 surahs")), ("Saving for offline use · 37 of 114 surahs".into(), &[("Cancel", Click::Stop)][..]));
        assert_eq!(text(&saving("fonts 212 of 604")).0, "Saving for offline use · fonts 212 of 604");
        assert_eq!(text(&Note::Saving(Kind::Update, None)).0, "Updating the offline copy…");
        assert_eq!(text(&Note::Saved(Kind::Download)), ("Saved for offline use".into(), &[][..]));
        let (line, buttons) = text(&Note::Failed(Kind::Download, "quran.com can't be reached".into()));
        assert_eq!((line.as_str(), buttons.len()), ("Saving for offline use stopped: quran.com can't be reached", 2));
    }
}
