//! Keeping the line being read in view: the view moves when the line leaves the band from an eighth to three fifths of
//! the way down, back to 38 %, briskly, and stays put while the reader reads lines in view. Coming to a line's end with
//! the next line not fully in view, it moves before the reader gets there, timed from their pace. Scrolling
//! continuously instead (experimental: some find it interrupting), the place being read, taken as the share of its
//! line read on toward the next line, is kept a third of the way down, eased over 0.8 s, so the page moves a line for
//! each line read (in the user's recitations a line takes about 6 s at 1.3 words a second, and pauses at ayah ends
//! stop it), and a move of the reader's own is left alone until they are heard on another line.

use egui::Rect;

use crate::reader::Slot;

/// How far down the line being read is kept: the band it may move in and where it is put back; scrolling continuously,
/// where it is kept; and the time constants (s).
const BAND: (f32, f32) = (0.12, 0.6);
const BACK_TO: f32 = 0.38;
const KEPT_AT: f32 = 1.0 / 3.0;
const BRISK: f32 = 0.35;
const FLOW: f32 = 0.8;
/// Seconds before the line's end at which the next line must be in view, and the room left under it (points).
const AHEAD: f32 = 1.5;
const ROOM: f32 = 8.0;

#[derive(Debug, Default)]
pub struct Keep {
    /// Moving the view back into the band.
    scrolling: bool,
    /// The reader moved the view themselves while reading this line (its first word): it stays until they are heard
    /// on another.
    left_alone: Option<(u16, u16, u16)>,
    /// Seconds after the last frame at which the scroll has gone on by half a pixel, with no news.
    next: Option<f32>,
    /// The view did not move when asked to with this line being read (its first word): at an end of what it shows, it
    /// is left as it is until another line is read there.
    stuck: Option<(u16, u16, u16)>,
}

/// The line word `i` is on: its first and last word.
pub fn line_of(words: &[Slot], i: usize) -> (usize, usize) {
    let y = words[i].1.center().y;
    let on_line = |j: usize| (words[j].1.center().y - y).abs() < words[i].1.height() * 0.5;
    let (mut a, mut b) = (i, i);
    while a > 0 && on_line(a - 1) {
        a -= 1;
    }
    while b + 1 < words.len() && on_line(b + 1) {
        b += 1;
    }
    (a, b)
}

/// Left and right edges of the words `a..=b`.
pub fn edges(words: &[Slot], (a, b): (usize, usize)) -> (f32, f32) {
    let span = words[a..=b].iter().map(|(_, r, ..)| (r.left(), r.right()));
    span.fold((f32::INFINITY, f32::NEG_INFINITY), |(l, r), (a, b)| (l.min(a), r.max(b)))
}

impl Keep {
    /// The reader moved the view themselves.
    pub fn reader_moved(&mut self, words: &[Slot], i: Option<usize>) {
        self.left_alone = i.map(|i| words[line_of(words, i).0].0);
        self.stuck = None;
    }

    /// The view did not scroll as asked last frame, word `i` being read: it can go no further that way.
    pub fn stuck(&mut self, words: &[Slot], i: usize) {
        self.stuck = Some(words[line_of(words, i).0].0);
        self.scrolling = false;
    }

    /// How far to scroll this frame (points, down) with the dot at `y` on word `i` of `words`, `share` of it read, in
    /// `area`, `dt` s after the last frame, the reader reading `pace` words a second; `continuous`: scroll continuously.
    /// Less than half a pixel (`px`) left to go is left: it would never show.
    #[allow(clippy::too_many_arguments)]
    pub fn scroll(&mut self, words: &[Slot], i: usize, share: f32, y: f32, area: Rect, dt: f32, pace: f32, continuous: bool, px: f32) -> f32 {
        self.next = None;
        let (a, b) = line_of(words, i);
        if self.stuck.is_some_and(|first| first == words[a].0) {
            return 0.0;
        }
        self.stuck = None;
        let at = |k: f32| area.top() + area.height() * k;
        // The next line's bottom below the view, once the reader is due at this line's end within `AHEAD` s (or in its
        // last word, their pace unknown): how much higher the line must stand.
        let left = (b - i) as f32 + 1.0 - share;
        let due = if pace > 0.1 { left / pace <= AHEAD } else { left <= 1.0 };
        let hidden = match words.get(b + 1) {
            Some(next) => next.1.bottom() + ROOM - area.bottom(),
            None => words[i].1.height() * 2.0 + ROOM - (area.bottom() - words[i].1.bottom()),
        };
        let lift = (due && hidden > 0.0).then_some(hidden);
        let floor = at(BAND.0);
        if continuous {
            if self.left_alone.is_some_and(|first| first == words[a].0) {
                return 0.0;
            }
            self.left_alone = None;
            let (l, r) = edges(words, (a, b));
            let read = ((r - words[i].1.right() + share * words[i].1.width()) / (r - l).max(1.0)).clamp(0.0, 1.0);
            let line = words[i].1.center().y;
            let pitch = words.get(b + 1).map(|w| w.1.center().y - line).filter(|p| *p > 0.0).unwrap_or(words[i].1.height());
            let want = lift.map_or(at(KEPT_AT), |h| (y - h).max(floor).min(at(KEPT_AT)));
            return self.ease(y + read * pitch - want, FLOW, dt, px);
        }
        let want = lift.map_or(at(BACK_TO), |h| (y - h).max(floor).min(at(BACK_TO)));
        if y > at(BAND.1) || y < floor || lift.is_some() {
            self.scrolling = true;
        }
        if !self.scrolling {
            return 0.0;
        }
        self.scrolling = (y - want).abs() > 2.0;
        let step = self.ease(y - want, BRISK, dt, px);
        if !self.scrolling {
            self.next = None;
        }
        step
    }

    /// Seconds after the last frame at which the scroll will have gone on by half a pixel, with no news (none: it has
    /// stopped).
    pub fn wake(&self) -> Option<f32> {
        self.next
    }

    /// This frame's step of `gap` (points) eased over `tau` s, `dt` after the last: none for less than half a pixel
    /// (`px`); and when the next half pixel of it is due.
    fn ease(&mut self, gap: f32, tau: f32, dt: f32, px: f32) -> f32 {
        if gap.abs() < px.max(1e-6) {
            return 0.0;
        }
        let step = gap * (1.0 - (-dt / tau).exp());
        let left = (gap - step).abs();
        self.next = (left > px).then(|| -tau * (1.0 - px / left).ln());
        step
    }
}

#[cfg(test)]
mod tests {
    use egui::{pos2, vec2};

    use super::*;

    /// Lines of eight words 50 wide, right to left, `pitch` apart from y = `top`; words 1.. of 2:7.
    fn lines(top: f32, pitch: f32, n: u16) -> Vec<Slot> {
        (0..n * 8)
            .map(|k| {
                let (line, col) = (f32::from(k / 8), f32::from(k % 8));
                let rect = egui::Rect::from_min_size(pos2(400.0 - 50.0 * col, top + line * pitch), vec2(48.0, 40.0));
                ((2, 7, k + 1), rect, rect.top() + 28.0, 20.0)
            })
            .collect()
    }

    fn area() -> Rect {
        Rect::from_min_size(pos2(0.0, 0.0), vec2(500.0, 600.0))
    }

    #[test]
    fn the_view_moves_only_once_the_line_leaves_the_band() {
        let (words, mut keep) = (lines(100.0, 60.0, 6), Keep::default());
        // Mid-line on the second line (y 184): in the band, no move.
        assert_eq!(keep.scroll(&words, 11, 0.5, 184.0, area(), 1.0 / 60.0, 1.3, false, 0.5), 0.0);
        // Below three fifths: back toward 38 %.
        let words = lines(400.0, 60.0, 6);
        assert!(keep.scroll(&words, 3, 0.5, 424.0, area(), 1.0 / 60.0, 1.3, false, 0.5) > 0.0);
    }

    #[test]
    fn scrolling_continuously_the_view_flows_a_line_for_each_line_read() {
        // Reading the second line from its start to its end, the line kept a third of the way down (200): the scroll
        // over the line adds up to its pitch (less the lag of the easing), in even steps.
        let (pitch, mut keep, mut scrolled) = (60.0, Keep::default(), 0.0f32);
        let mut steps = Vec::new();
        for k in 0..480 {
            let read = k as f32 / 480.0 * 8.0;
            let i = 8 + read as usize;
            let words = lines(116.0 - scrolled, pitch, 4);
            let y = words[i].2 - 4.0;
            let step = keep.scroll(&words, i, read.fract(), y, area(), 1.0 / 60.0, 1.3, true, 0.5);
            scrolled += step;
            steps.push(step);
        }
        assert!((scrolled - pitch).abs() < 10.0, "{scrolled}");
        assert!(steps[60..].windows(2).all(|w| (w[1] - w[0]).abs() < 0.05), "no jumps: {steps:?}");
    }

    #[test]
    fn at_a_lines_end_the_next_line_comes_into_view_before_the_reader_gets_there() {
        // The line at the band's foot (y 340, inside three fifths), its next line under the view's bottom: in the
        // line's last words at 1.3 words a second the view moves; early in the line it waits.
        let words = lines(320.0, 260.0, 2);
        assert_eq!(Keep::default().scroll(&words, 1, 0.5, 344.0, area(), 1.0 / 60.0, 1.3, false, 0.5), 0.0);
        for continuous in [false, true] {
            let mut keep = Keep::default();
            let (mut y, mut moved) = (344.0, 0.0);
            for _ in 0..120 {
                let step = keep.scroll(&words, 6, 0.6, y, area(), 1.0 / 60.0, 1.3, continuous, 0.5);
                (y, moved) = (y - step, moved + step);
            }
            assert!(words[8].1.bottom() - moved + ROOM <= area().bottom() + 1.0, "continuous {continuous}: moved {moved}");
        }
    }

    #[test]
    fn a_move_of_the_readers_own_is_left_alone_until_the_next_line() {
        let (words, mut keep) = (lines(500.0, 60.0, 3), Keep::default());
        keep.reader_moved(&words, Some(3));
        assert_eq!(keep.scroll(&words, 5, 0.5, 524.0, area(), 1.0 / 60.0, 1.3, true, 0.5), 0.0);
        assert!(keep.scroll(&words, 9, 0.2, 584.0, area(), 1.0 / 60.0, 1.3, true, 0.5) > 0.0, "heard on the next line");
    }

    #[test]
    fn a_scroll_steps_by_half_a_pixel_when_due_and_stops_with_less_left() {
        // Scrolling continuously with the reader stopped mid-line, frames only when the scroll asks: the view eases
        // to where the line is kept, each later step half a pixel, and stops with less than that left.
        let words = lines(116.0, 60.0, 4);
        let (mut keep, mut moved, mut dt, mut steps) = (Keep::default(), 0.0f32, 1.0 / 60.0, Vec::new());
        loop {
            let here: Vec<Slot> = words.iter().map(|&(l, r, b, e)| (l, r.translate(vec2(0.0, -moved)), b - moved, e)).collect();
            let step = keep.scroll(&here, 10, 0.5, here[10].2 - 4.0, area(), dt, 1.3, true, 0.5);
            (moved, dt) = (moved + step, keep.wake().unwrap_or(f32::INFINITY));
            steps.push(step);
            assert!(steps.len() < 200, "{steps:?}");
            if dt.is_infinite() {
                break;
            }
        }
        assert!(steps.len() > 20 && steps[5..].iter().all(|s| (s - 0.5).abs() < 0.01), "{steps:?}");
        let here: Vec<Slot> = words.iter().map(|&(l, r, b, e)| (l, r.translate(vec2(0.0, -moved)), b - moved, e)).collect();
        assert_eq!(keep.scroll(&here, 10, 0.5, here[10].2 - 4.0, area(), 1.0, 1.3, true, 0.5), 0.0, "at rest");
    }

    #[test]
    fn a_view_that_did_not_move_is_left_as_it_is_until_the_next_line() {
        // Low in a view at the end of what it shows (asked to scroll, it did not): no more asking on this line.
        let (words, mut keep) = (lines(400.0, 60.0, 6), Keep::default());
        assert!(keep.scroll(&words, 3, 0.5, 424.0, area(), 1.0 / 60.0, 1.3, false, 0.5) > 0.0);
        keep.stuck(&words, 3);
        assert_eq!(keep.scroll(&words, 5, 0.5, 424.0, area(), 1.0 / 60.0, 1.3, false, 0.5), 0.0);
        assert_eq!(keep.wake(), None);
        assert!(keep.scroll(&words, 9, 0.5, 484.0, area(), 1.0 / 60.0, 1.3, false, 0.5) > 0.0, "the next line");
    }

    #[test]
    fn reading_lines_in_view_mid_page_never_moves_the_view() {
        // Six lines from a fifth of the way down, the first four read word by word at 1.3 words a second, each next
        // line in view: nothing moves.
        let (words, mut keep) = (lines(120.0, 60.0, 6), Keep::default());
        for i in 0..32 {
            for k in 0..40 {
                let y = words[i].2 - 4.0;
                assert_eq!(keep.scroll(&words, i, k as f32 / 40.0, y, area(), 1.0 / 60.0, 1.3, false, 0.5), 0.0, "word {i}");
            }
        }
    }
}
