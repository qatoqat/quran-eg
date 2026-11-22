//! The voice pointer's toast, in the header's look (its colours, border and type): where it is about to take the
//! reader (a far place, counted down, with Go now and Stay), a warning for a place that sounds like the ayah being read
//! (it waits for Go there or Stay), the words the reading went past unread (Go back to them, or Dismiss the marks), or a
//! note that the microphone hears the voice only faintly (Dismiss). One line with
//! its buttons at the end, in a capsule like the header's Surah | Juz | Page: the one that stands for doing nothing is
//! filled. It sits a little below the line being read, where the eyes are (three quarters of the way down while that
//! line is not known), and stays put: it never slides with the page, and moves only when the line being read comes
//! onto it or goes far from it (the next line, a page turned).

use std::sync::Arc;

use egui::{Color32, CornerRadius, FontId, Galley, Id, LayerId, Order, Rect, Sense, Stroke, StrokeKind, UiBuilder, Vec2, pos2, vec2};
use quran_core::api::Chapter;
use quran_core::lists::find_chapter;
use quran_core::reference::words_in;

use super::follow::Offer;
use super::link::Loc;
use super::skipped::Summary;
use crate::kit::Capsule;
use crate::theme::Colors;

/// What the toast says.
#[derive(Debug, Clone, PartialEq)]
pub enum Note {
    /// A far place, and the seconds left before the reader is taken there.
    Moving(Loc, f32),
    /// A far place that sounds like the ayah being read.
    Alike(Loc),
    /// Words the reading went past unread (the first run of them).
    Skipped(Summary),
    /// The voice stands only a little over the room's sound.
    Weak,
}

impl Note {
    pub fn of(offer: Offer) -> Note {
        offer.left.map_or(Note::Alike(offer.to), |left| Note::Moving(offer.to, left))
    }

    /// What it is about (a place, or the microphone), whatever its countdown says.
    fn about(&self) -> (Option<Loc>, bool) {
        match *self {
            Note::Moving(to, _) | Note::Alike(to) => (Some(to), false),
            Note::Skipped(s) => (Some(s.from), false),
            Note::Weak => (None, true),
        }
    }

    /// The one toast shown: a far place's before the words gone past unread, those before the faint microphone.
    pub fn first(offer: Option<Offer>, skipped: Option<Summary>, weak: bool) -> Option<Note> {
        offer.map(Note::of).or(skipped.map(Note::Skipped)).or(weak.then_some(Note::Weak))
    }
}

/// A button clicked on the toast.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Click {
    Go,
    Stay,
    /// To the words gone past unread.
    Back,
    Dismiss,
}

/// The ayah, after its surah's name where the lists give it.
fn place(chapters: &[Chapter], (s, a, _): Loc) -> String {
    find_chapter(chapters, s).map_or_else(|| format!("{s}:{a}"), |c| format!("{} {s}:{a}", c.name_simple))
}

/// The words gone past unread: an ayah or ayahs whole (`Al-Qalam 68:44`, `Al-Qalam 68:44–45`), else so many words in
/// them, and how many places more.
fn skipped(chapters: &[Chapter], s: &Summary) -> String {
    let whole = s.from.2 == 1 && words_in(s.to.0, s.to.1).is_some_and(|n| n == usize::from(s.to.2));
    let ayahs = match (s.from, s.to) {
        (a, b) if (a.0, a.1) == (b.0, b.1) => place(chapters, a),
        (a, b) if a.0 == b.0 => format!("{}–{}", place(chapters, a), b.1),
        (a, b) => format!("{} – {}:{}", place(chapters, a), b.0, b.1),
    };
    let what = match (whole, s.words) {
        (true, _) => ayahs,
        (false, 1) => format!("a word in {ayahs}"),
        (false, n) => format!("{n} words in {ayahs}"),
    };
    match s.others {
        0 => format!("Skipped {what}"),
        n => format!("Skipped {what} · {n} more"),
    }
}

/// The toast's line, its buttons and what each does, and the one that stands for doing nothing.
pub fn text(note: &Note, chapters: &[Chapter]) -> (String, &'static [(&'static str, Click)], Option<usize>) {
    match *note {
        Note::Moving(to, left) => {
            let when = if left >= 0.5 { format!("in {:.0} s", left.ceil()) } else { "now".into() };
            (format!("Moving to {} {when}", place(chapters, to)), &[("Go now", Click::Go), ("Stay", Click::Stay)], Some(0))
        }
        Note::Alike(to) => (format!("Sounds like {}", place(chapters, to)), &[("Go there", Click::Go), ("Stay", Click::Stay)], Some(1)),
        Note::Skipped(ref s) => (skipped(chapters, s), &[("Go back", Click::Back), ("Dismiss", Click::Dismiss)], None),
        Note::Weak => ("Mic is faint: hold it closer".into(), &[("Dismiss", Click::Dismiss)], None),
    }
}

/// The type: the header's name (its title is 16.8).
const TEXT: f32 = 16.0;
/// The padding: with the capsule's height, a one-line toast is as tall as the header.
const PAD: Vec2 = vec2(16.0, 10.0);
/// Between the line and the buttons.
const GAP: f32 = 16.0;
const RADIUS: f32 = 12.0;

/// A toast laid out: sized before it is placed, then shown.
pub struct Toast {
    line: Arc<Galley>,
    buttons: Capsule,
    clicks: &'static [(&'static str, Click)],
    on: Option<usize>,
    size: Vec2,
    scale: f32,
}

impl Toast {
    /// `note` at `scale` times the page's toast, at most `width` wide (its line wraps first).
    pub fn new(ctx: &egui::Context, note: &Note, chapters: &[Chapter], colors: &Colors, width: f32, scale: f32) -> Toast {
        let (line, clicks, on) = text(note, chapters);
        let painter = ctx.layer_painter(LayerId::new(Order::Foreground, Id::unique("voice-toast-layout")));
        let labels: Vec<&str> = clicks.iter().map(|c| c.0).collect();
        let buttons = Capsule::new(&painter, &labels, scale, f32::INFINITY);
        let (pad, gap, room) = (PAD * scale, GAP * scale, buttons.size());
        let wrap = (width - 2.0 * pad.x - gap - room.x).max(60.0 * scale);
        let line = painter.layout(line, FontId::proportional(TEXT * scale), colors.fg, wrap);
        let size = vec2(line.size().x + gap + room.x, line.size().y.max(room.y)) + 2.0 * pad;
        Toast { line, buttons, clicks, on, size, scale }
    }

    pub fn height(&self) -> f32 {
        self.size.y
    }

    /// Shows the toast centred in `area`, its top at `y`; returns the button clicked.
    pub fn show(&self, ctx: &egui::Context, id: Id, area: Rect, y: f32, colors: &Colors) -> Option<Click> {
        let at = pos2((area.center().x - self.size.x / 2.0).max(area.left()), y);
        let area = egui::Area::new(id).order(Order::Foreground).fixed_pos(at);
        let clicked = area.show(ctx, |ui| {
            let (rect, _) = ui.allocate_exact_size(self.size, Sense::hover());
            let radius = CornerRadius::same((RADIUS * self.scale) as u8);
            let shadow = egui::Shadow { offset: [0, 3], blur: 14, spread: 0, color: Color32::from_black_alpha(40) };
            ui.painter().add(shadow.as_shape(rect, radius));
            ui.painter().rect(rect, radius, colors.card, Stroke::new(1.0, colors.border), StrokeKind::Inside);
            let inner = rect.shrink2(PAD * self.scale);
            let line = pos2(inner.left(), inner.center().y - self.line.size().y / 2.0);
            ui.painter().galley(line, self.line.clone(), colors.fg);
            let room = self.buttons.size();
            let buttons = Rect::from_min_size(pos2(inner.right() - room.x, inner.center().y - room.y / 2.0), room);
            ui.scope_builder(UiBuilder::new().max_rect(buttons), |ui| self.buttons.show(ui, colors, self.on).clicked()).inner
        });
        clicked.inner.map(|i| self.clicks[i].1)
    }
}

/// Where the toast is: placed when it appears, and kept there while it shows.
#[derive(Debug, Default)]
pub struct Pin {
    /// What the toast shown is about, and its top.
    at: Option<((Option<Loc>, bool), f32)>,
}

impl Pin {
    /// The top of a toast `height` tall saying `note` in `area`, the line being read at `line` (its middle and height).
    pub fn top(&mut self, note: Option<&Note>, area: Rect, line: Option<(f32, f32)>, height: f32) -> Option<f32> {
        let Some(note) = note else {
            self.at = None;
            return None;
        };
        let fresh = top(area, line, height);
        let y = match (self.at, line) {
            (Some((about, y)), Some(line)) if about == note.about() && !moves(y, height, fresh, line) => y,
            (Some((about, y)), None) if about == note.about() => y,
            _ => fresh,
        };
        self.at = Some((note.about(), y));
        Some(y.clamp(area.top(), (area.bottom() - height).max(area.top())))
    }
}

/// Where the toast's top goes in `area`: under the line being read (its middle `line` and height), or above it when
/// there is no room below; three quarters down without a line.
fn top(area: Rect, line: Option<(f32, f32)>, height: f32) -> f32 {
    match line {
        Some((y, h)) if y + 0.9 * h + height <= area.bottom() - 4.0 => y + 0.9 * h,
        Some((y, h)) => (y - 0.9 * h - height).max(area.top()),
        None => (area.top() + area.height() * 0.75).min(area.bottom() - height - 4.0),
    }
}

/// Whether a toast at `y`, `height` tall, goes to `fresh` (its place by the line being read, whose middle is `l` and
/// height `h`): when that line comes onto it, or is more than a few lines from it.
fn moves(y: f32, height: f32, fresh: f32, (l, h): (f32, f32)) -> bool {
    let covered = l + 0.5 * h > y && l - 0.5 * h < y + height;
    covered || (y - fresh).abs() > 3.0 * h
}

#[cfg(test)]
mod tests {
    use egui::pos2;

    use super::*;

    #[test]
    fn the_toast_says_where_and_when_in_a_line() {
        let (line, buttons, on) = text(&Note::Moving((2, 95, 1), 3.2), &[]);
        assert_eq!((line.as_str(), buttons, on), ("Moving to 2:95 in 4 s", &[("Go now", Click::Go), ("Stay", Click::Stay)][..], Some(0)));
        assert_eq!(text(&Note::Moving((2, 95, 1), 0.2), &[]).0, "Moving to 2:95 now");
        let (line, buttons, on) = text(&Note::of(Offer { to: (62, 7, 3), left: None }), &[]);
        assert_eq!((line.as_str(), buttons, on), ("Sounds like 62:7", &[("Go there", Click::Go), ("Stay", Click::Stay)][..], Some(1)));
        let (line, buttons, on) = text(&Note::Weak, &[]);
        assert_eq!((line.as_str(), buttons, on), ("Mic is faint: hold it closer", &[("Dismiss", Click::Dismiss)][..], None));
    }

    #[test]
    fn the_skipped_words_toast_names_them_and_offers_to_go_back_or_dismiss() {
        let note = |from, to, words, others| Note::Skipped(Summary { from, to, words, others });
        // 68:45 has 5 words, 68:44 10.
        let (line, buttons, on) = text(&note((68, 45, 1), (68, 45, 5), 5, 0), &[]);
        assert_eq!((line.as_str(), buttons, on), ("Skipped 68:45", &[("Go back", Click::Back), ("Dismiss", Click::Dismiss)][..], None));
        assert_eq!(text(&note((68, 44, 4), (68, 44, 6), 3, 0), &[]).0, "Skipped 3 words in 68:44");
        assert_eq!(text(&note((68, 44, 9), (68, 44, 9), 1, 2), &[]).0, "Skipped a word in 68:44 · 2 more");
        assert_eq!(text(&note((68, 44, 1), (68, 45, 5), 15, 0), &[]).0, "Skipped 68:44–45");
        assert_eq!(text(&note((68, 44, 3), (68, 45, 5), 13, 0), &[]).0, "Skipped 13 words in 68:44–45");
        assert_eq!(text(&note((68, 52, 1), (69, 1, 1), 6, 0), &[]).0, "Skipped 68:52 – 69:1");
        // The far place's toast and the look-alike's come first, the faint microphone's after.
        let offer = Offer { to: (2, 95, 1), left: Some(3.0) };
        let summary = Summary { from: (68, 45, 1), to: (68, 45, 5), words: 5, others: 0 };
        assert_eq!(Note::first(Some(offer), Some(summary), true), Some(Note::of(offer)));
        assert_eq!(Note::first(None, Some(summary), true), Some(Note::Skipped(summary)));
        assert_eq!((Note::first(None, None, true), Note::first(None, None, false)), (Some(Note::Weak), None));
    }

    #[test]
    fn the_toast_sits_under_the_line_and_never_over_it() {
        let area = Rect::from_min_max(pos2(0.0, 0.0), pos2(400.0, 800.0));
        // Under the line, a little below it.
        assert_eq!(top(area, Some((300.0, 60.0)), 50.0), 354.0);
        // No room below: above it, clear of the line.
        let t = top(area, Some((760.0, 60.0)), 50.0);
        assert!(t + 50.0 <= 760.0 - 30.0, "{t}");
        // No line known: three quarters down.
        assert_eq!(top(area, None, 50.0), 600.0);
    }

    #[test]
    fn a_shown_toast_stays_put_until_the_line_read_comes_onto_it_or_goes_far() {
        let area = Rect::from_min_max(pos2(0.0, 50.0), pos2(400.0, 850.0));
        let mut pin = Pin::default();
        // The microphone's too, under the line being read.
        let y = pin.top(Some(&Note::Weak), area, Some((300.0, 60.0)), 50.0).unwrap();
        assert_eq!(y, 354.0);
        // The line a little lower, or not known (the reader scrolled): it stays.
        assert_eq!(pin.top(Some(&Note::Weak), area, Some((320.0, 60.0)), 50.0), Some(y));
        assert_eq!(pin.top(Some(&Note::Weak), area, None, 50.0), Some(y));
        // The next line comes onto it: under that one.
        assert_eq!(pin.top(Some(&Note::Weak), area, Some((390.0, 60.0)), 50.0), Some(444.0));
        // A page turned, the line read far up: under it there.
        assert_eq!(pin.top(Some(&Note::Weak), area, Some((120.0, 60.0)), 50.0), Some(174.0));
        // A far place's countdown keeps its toast; another place is placed anew; gone, nothing.
        let y = pin.top(Some(&Note::Moving((2, 95, 1), 4.0)), area, Some((500.0, 60.0)), 50.0).unwrap();
        assert_eq!((y, pin.top(Some(&Note::Moving((2, 95, 1), 3.0)), area, Some((510.0, 60.0)), 50.0)), (554.0, Some(554.0)));
        assert_eq!(pin.top(Some(&Note::Alike((62, 7, 3))), area, Some((510.0, 60.0)), 50.0), Some(564.0));
        assert_eq!(pin.top(None, area, None, 50.0), None);
    }
}
