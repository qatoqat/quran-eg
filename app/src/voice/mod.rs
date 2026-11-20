//! The voice pointer (native builds): quran-voice's listener hears the recitation and a dot on the
//! letter being read moves across the mushaf, inside an accuracy range (a rounded rectangle over the
//! words it could be on), as the quran.com extension does. `link` talks to the listener; `motion`
//! smooths its bursty updates into a glide at the reader's pace, each part of the pointer easing as
//! `glide` does; `follow` decides when the reader goes to a place heard off screen; `meter` shows on
//! the Listen button how the microphone hears.

mod follow;
mod glide;
mod link;
mod meter;
mod motion;

use egui::epaint::RectShape;
use egui::{Color32, CornerRadius, Pos2, Rect, Stroke, StrokeKind, pos2, vec2};
use quran_core::settings::Voice as Settings;
use quran_core::settings::palette::parse_hex;
use quran_core::view::WordRef;

use self::follow::{Pages, Verdict};
use self::link::{Event, Heard, Link};
use self::meter::Meter;
use self::motion::{Follower, Jump, Params, Reading};
use crate::header::Listening;
use crate::kit::Mic;
use crate::pointer::{Lens, ReadingPointer};
use crate::theme::Colors;

pub use self::glide::CURVES;
pub use self::link::Loc;

use crate::reader::Slot;

/// Seconds a new or jumping pointer takes to fade in.
const FADE: f32 = 0.25;
/// Seconds the connecting spinner takes to turn once.
const SPIN: f64 = 1.6;

#[derive(Debug, Clone, PartialEq)]
pub enum Status {
    Starting,
    Listening,
    Failed(String),
}

/// What the reader should do for the voice pointer.
#[derive(Debug, Default, PartialEq)]
pub struct Steer {
    /// Scroll by this much, to keep the line being read in view.
    pub scroll: f32,
    /// The place being read is not on screen: go there.
    pub go_to: Option<WordRef>,
    /// The reader closed the reading with ṣadaqa-llāhu-l-ʿaẓīm.
    pub closed: bool,
}

pub struct Voice {
    link: Link,
    status: Status,
    follower: Follower,
    dot: ReadingPointer,
    last: Option<Heard>,
    /// Fade-in of a pointer that has just appeared or jumped, 0..=1.
    fade: f32,
    line_y: Option<f32>,
    /// Where the dot was drawn last, and its line's height (the glow's lens follows it).
    drawn: Option<(Pos2, f32)>,
    /// The words on screen as last sent to the listener, and when.
    sent: (Option<(Loc, Loc)>, f64),
    /// The pointer has scrolled or gone somewhere since: the words sent next are its doing, not the
    /// reader's choice, and must not draw the listener's search (after a wrong lock, the wrong page).
    steered: bool,
    /// When to take the reader to a place heard off screen.
    pages: Pages,
    scrolling: bool,
    /// The reader has just closed with ṣadaqa-llāhu-l-ʿaẓīm (until the next frame reports it).
    closed: bool,
    meter: Meter,
}

/// The listener's formulas outside the mushaf (surah 0), by number, for the title bar.
fn formula(ayah: u16) -> &'static str {
    match ayah {
        1 => "isti'adha",
        2 => "basmala",
        3 | 4 => "sadaqa-llahu-l-'azim",
        _ => "takbir",
    }
}

/// The last word of ṣadaqa-llāhu-l-ʿaẓīm (0:3) or of its form with al-ʿaliyy (0:4).
fn closes(loc: Loc) -> bool {
    matches!(loc, (0, 3, 3) | (0, 4, 4))
}

/// `#rrggbb` at an opacity.
fn rgba(hex: &str, alpha: f32) -> Color32 {
    let [r, g, b] = parse_hex(hex).unwrap_or([255, 140, 0]);
    Color32::from_rgba_unmultiplied(r, g, b, (alpha.clamp(0.0, 1.0) * 255.0).round() as u8)
}

fn reading(state: &str) -> Option<Reading> {
    Some(match state {
        "Locked" => Reading::Locked,
        "Tentative" => Reading::Tentative,
        "Drifting" => Reading::Drifting,
        "Paused" => Reading::Paused,
        _ => return None,
    })
}

/// The point at word position `c` (the index of a word in the whole Quran plus the share of it read,
/// right to left), `lift` (a share of the text size) above the baseline: the word heard last anchors
/// the index among the words on screen, which are in reading order. Also returns the word's index in
/// `words`.
fn point(c: f32, anchor: &Heard, words: &[Slot], lift: f32) -> Option<(Pos2, usize)> {
    let at = words.binary_search_by(|(l, ..)| l.cmp(&anchor.loc)).ok()?;
    let i = usize::try_from(at as i64 + c.floor() as i64 - i64::from(anchor.word)).ok()?;
    let (_, rect, baseline, em) = words.get(i)?;
    let f = (c - c.floor()).clamp(0.0, 1.0);
    Some((pos2(rect.right() - f * rect.width(), baseline - lift * em), i))
}

/// The words on the line word `i` is on: the first's and the last's index.
fn line_of(words: &[Slot], i: usize) -> (usize, usize) {
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
fn edges(words: &[Slot], (a, b): (usize, usize)) -> (f32, f32) {
    let span = words[a..=b].iter().map(|(_, r, ..)| (r.left(), r.right()));
    span.fold((f32::INFINITY, f32::NEG_INFINITY), |(l, r), (a, b)| (l.min(a), r.max(b)))
}

/// Where the accuracy range is drawn, its edges at word positions `start` and `end` and the pointer at
/// `at` on word `i`: on the pointer's line from `end` to `start` (to the line's end where an edge lies
/// off it; reading runs right to left), and, when `start` lies on the line before (the range reaching
/// back to a reader heard there while the pointer waits), there too, from that line's left end to
/// `start`. `place` finds a position's point and word; `size` is the height, the padding beyond the
/// words and the least width.
fn range_rects(
    words: &[Slot],
    place: impl Fn(f32) -> Option<(Pos2, usize)>,
    (start, end): (f32, f32),
    (at, i): (Pos2, usize),
    (height, pad, least): (f32, f32, f32),
) -> Vec<Rect> {
    let line = line_of(words, i);
    let (left_end, right_end) = edges(words, line);
    let near = words[i].1.height() * 0.5;
    let on_line = |c: f32| place(c).filter(|(q, _)| (q.y - at.y).abs() < near).map(|(q, _)| q.x);
    let right = on_line(start).unwrap_or(right_end).min(right_end);
    let left = on_line(end).unwrap_or(left_end).max(left_end);
    let width = (right - left + 2.0 * pad).max(least);
    let mut rects = vec![Rect::from_min_size(pos2(left - pad, at.y - height / 2.0), vec2(width, height))];
    if let Some((q, j)) = place(start)
        && line_of(words, j).1 + 1 == line.0
    {
        let (before_left, _) = edges(words, line_of(words, j));
        rects.push(Rect::from_min_max(pos2(before_left - pad, q.y - height / 2.0), pos2(q.x + pad, q.y + height / 2.0)));
    }
    rects
}

/// `s:a:w`.
fn key(loc: Loc) -> String {
    format!("{}:{}:{}", loc.0, loc.1, loc.2)
}

/// Seconds since the last frame, for what moves: egui's steady measure, which takes the first frame
/// after the app slept (nothing moving, no news) for one frame, not the whole sleep. A wait or a pause
/// that ends after a sleep starts its glide or fade then, instead of finishing it in one step.
fn frame_dt(ctx: &egui::Context) -> f32 {
    ctx.input(|i| i.stable_dt).min(0.25)
}

/// The pointer's fade `dt` on: out (parked at an ayah's end) or in.
fn fade_step(fade: f32, out: bool, dt: f32) -> f32 {
    if out { (fade - dt / FADE).max(0.0) } else { (fade + dt / FADE).min(1.0) }
}

/// Frames only while something moves (`settled`: the pointer, the range's edges, the fade and the
/// scroll are at rest); a wait or a pause that is to end asks for one frame then. The listener's news
/// wakes the app otherwise.
fn ask_frame(ctx: &egui::Context, follower: &Follower, settled: bool, now: f64, params: &Params) {
    if !settled {
        ctx.request_repaint();
    } else if let Some(after) = follower.wake(now, params) {
        ctx.request_repaint_after_secs(after);
    }
}

impl Voice {
    /// Joins quran-voice's listener, or starts the installed one.
    pub fn start(settings: &Settings, ctx: &egui::Context) -> Result<Voice, String> {
        let link = Link::start(link::folder(&settings.listener), ctx.clone())?;
        Ok(Voice {
            link,
            status: Status::Starting,
            follower: Follower::default(),
            dot: ReadingPointer::default(),
            last: None,
            fade: 0.0,
            line_y: None,
            drawn: None,
            sent: (None, 0.0),
            steered: false,
            pages: Pages::default(),
            scrolling: false,
            closed: false,
            meter: Meter::default(),
        })
    }

    pub fn status(&self) -> &Status {
        &self.status
    }

    /// One line for the title bar: what the listener hears (or that the microphone does not).
    pub fn summary(&self) -> String {
        match self.meter.trouble().filter(|_| self.status == Status::Listening) {
            Some(trouble) => format!("◉ Listening · {trouble}"),
            None => self.heard_summary(),
        }
    }

    /// What the listener hears, or that it is starting or has stopped.
    fn heard_summary(&self) -> String {
        if self.status == Status::Listening
            && let Some(s) = self.pages.stray()
        {
            return format!("◉ Listening · lost the place ({}:{} heard elsewhere)", s.to.0, s.to.1);
        }
        match (&self.status, &self.last) {
            (Status::Starting, _) if self.link.started_it() => "◉ Starting the listener…".into(),
            (Status::Starting, _) => "◉ Joining the listener…".into(),
            (Status::Failed(e), _) => format!("Listening stopped: {e}"),
            (Status::Listening, None) => "◉ Listening: recite from any page".into(),
            (Status::Listening, Some(h)) if h.loc.0 == 0 => format!("◉ Listening · {} · {}", formula(h.loc.1), h.state),
            (Status::Listening, Some(h)) => format!("◉ Listening · {}:{} word {} · {}", h.loc.0, h.loc.1, h.loc.2, h.state),
        }
    }

    /// The Listen button: its microphone, and what its tooltip says.
    pub fn listening(&self, now: f64) -> Listening {
        let (mic, mic_line) = match self.status {
            Status::Starting => (Mic::Connecting((now / SPIN).fract() as f32), None),
            Status::Listening => (self.meter.mic(now), self.meter.line(now)),
            Status::Failed(_) => (Mic::Off, None),
        };
        // The microphone's trouble has its own line there.
        Listening { summary: self.heard_summary(), mic, mic_line }
    }

    /// The glow's lens at the dot, if it glows.
    pub fn lens(&self, settings: &Settings) -> Option<Lens> {
        let (at, line_height) = self.drawn?;
        self.dot.lens_at(&settings.pointer, at, line_height)
    }

    fn heard(&mut self, h: Heard, now: f64, params: &Params) {
        self.closed |= closes(h.loc) && h.state == "Locked";
        if !matches!(reading(&h.state), Some(Reading::Locked | Reading::Drifting | Reading::Paused)) {
            // Searching, or unsure: no longer followed well where it was.
            self.pages.lost();
        }
        match reading(&h.state).filter(|_| h.loc.0 > 0) {
            None => self.follower.hide(),
            Some(r) => {
                let frac = if h.letters > 0 { ((f32::from(h.letter) + 0.5) / f32::from(h.letters)).min(0.95) } else { 0.5 };
                let c = h.word as f32 + frac;
                if self.follower.update(c, r, h.conf, h.speed, h.t, now, params) == Jump::Respawn {
                    self.respawn();
                }
            }
        }
        self.last = Some(h);
    }

    /// The pointer appears afresh where it is going: it fades in, with nothing behind it to smear.
    fn respawn(&mut self) {
        self.fade = 0.0;
        self.line_y = None;
        self.dot.hide();
    }

    /// Reads the listener's news and moves the microphone's meter. Call before the header, which shows
    /// them.
    pub fn poll(&mut self, ctx: &egui::Context, settings: &Settings) {
        let now = ctx.input(|i| i.time);
        let params = Params::of(settings);
        for event in self.link.poll() {
            match event {
                Event::Connected => {
                    self.status = Status::Listening;
                    self.meter = Meter::default();
                }
                Event::Heard(h) => self.heard(h, now, &params),
                Event::Audio(a) => self.meter.hear(a, now),
                Event::Failed(e) => self.status = Status::Failed(e),
            }
        }
        // The meter asks for frames while it moves (nothing at rest); the connecting spinner turns.
        if let Some(after) = self.meter.tick(now) {
            ctx.request_repaint_after_secs(after);
        }
        if self.status == Status::Starting {
            ctx.request_repaint_after_secs(meter::FRAME);
        }
    }

    /// Moves and draws the pointer over `words` (the words on screen, in reading order) in `area`, and
    /// says how the reader should follow. Call after the reader has painted.
    pub fn frame(&mut self, ctx: &egui::Context, settings: &Settings, colors: &Colors, words: &[Slot], area: Rect) -> Steer {
        let now = ctx.input(|i| i.time);
        let dt = frame_dt(ctx);
        let params = Params::of(settings);
        // The words on screen are where the listener looks first.
        if let (Some(a), Some(b)) = (words.first(), words.last())
            && self.sent.0 != Some((a.0, b.0))
            && now - self.sent.1 > 1.0
        {
            self.link.send_visible(a.0, b.0, !std::mem::take(&mut self.steered));
            self.sent = (Some((a.0, b.0)), now);
        }
        let mut steer = Steer { closed: std::mem::take(&mut self.closed), ..Steer::default() };
        self.drawn = None;
        // The pointer runs on no further than the end of the ayah being read (readers stop there).
        if let Some(h) = &self.last
            && let Ok(at) = words.binary_search_by(|(l, ..)| l.cmp(&h.loc))
        {
            let rest = words[at + 1..].iter().take_while(|(l, ..)| (l.0, l.1) == (h.loc.0, h.loc.1)).count();
            self.follower.set_ayah_end(Some(h.word as f32 + rest as f32 + 0.95));
        }
        let (Some(anchor), Some(drawn)) = (self.last.clone(), self.follower.tick(now, dt, &params)) else { return steer };
        let state = reading(&anchor.state);
        let sure = state == Some(Reading::Locked) && anchor.conf > 0.8;
        let lift = f32::from(settings.lift) / 10.0;
        let place = |c: f32| point(c, &anchor, words, lift);
        let Some((at, i)) = place(drawn.x) else {
            // Not on screen: the reader goes there after a second, or, far from where the recitation
            // was being followed, once it has kept being followed there (lost until then). Hidden
            // meanwhile, the pointer fades in wherever it shows again.
            let far_wait = f64::from(settings.far_wait_ms) / 1000.0;
            if self.pages.off_screen(anchor.word, anchor.loc, sure, now, far_wait) == Verdict::Go && settings.auto_scroll {
                steer.go_to = Some(WordRef { chapter: anchor.loc.0, verse: anchor.loc.1, word: anchor.loc.2 });
            }
            if let Some(after) = self.pages.wake(now) {
                ctx.request_repaint_after_secs(after);
            }
            self.fade = 0.0;
            self.dot.hide();
            self.steered |= steer.go_to.is_some();
            return steer;
        };
        if let Some(slip) = self.pages.on_screen(anchor.word, anchor.loc, sure, now) {
            // Back from a far place: a slip, or the listener's mistake (where a mistake notifier would
            // say so).
            crate::stats::note(|| format!("voice: heard at {} for {:.1} s while followed at {}, then back", key(slip.to), now - slip.since, key(slip.from)));
        }
        let (h, em) = (words[i].1.height(), words[i].3);
        if self.line_y.is_some_and(|y| (at.y - y).abs() > h * 0.6) {
            self.respawn();
        }
        self.line_y = Some(at.y);
        // Parked at the ayah's end, the reader pausing there for the delay: faded out until they go on.
        let out = self.follower.faded(now, &params);
        self.fade = fade_step(self.fade, out, dt);
        let r = &settings.range;
        if r.enabled {
            let height = h * f32::from(r.size) / 10.0;
            let size = (height, h * 0.12, em * f32::from(settings.pointer.size) / 10.0);
            let opacity = f32::from(r.opacity) / 100.0;
            let radius = CornerRadius::same((height * f32::from(r.roundness) / 100.0).round().min(255.0) as u8);
            // An outline stays an outline: at most a third of the height, the fill keeps the rest.
            let outline = Stroke::new(f32::from(r.border).min(height / 3.0), rgba(&r.color, (opacity * 2.5 + 0.25).min(1.0) * self.fade));
            // Edge blur 10 softens it over half its height (epaint's feathering, as for shadows).
            let soft = height * f32::from(r.blur) * 0.05;
            let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Middle, egui::Id::unique("voice-range"))).with_clip_rect(area);
            for rect in range_rects(words, place, (drawn.start, drawn.end), (at, i), size) {
                painter.add(RectShape::filled(rect, radius, rgba(&r.color, opacity * self.fade)).with_blur_width(soft));
                if outline.width > 0.0 {
                    // Blurred no wider than itself, it keeps its width at any edge blur: with the fill's
                    // blur a thin outline would spread faintly over all of it, beyond the words.
                    painter.add(RectShape::stroke(rect, radius, outline, StrokeKind::Inside).with_blur_width(soft.min(outline.width)));
                }
            }
        }
        if settings.pointer.enabled {
            self.dot.show_at(ctx, &settings.pointer, colors, area, em, Some(f32::from(settings.ring)), at, self.fade);
            self.drawn = Some((at, em));
        }
        // Keep the line being read between an eighth and three fifths of the way down.
        if settings.auto_scroll && matches!(state, Some(Reading::Locked | Reading::Paused | Reading::Drifting)) {
            let want = area.top() + area.height() * 0.38;
            if at.y > area.top() + area.height() * 0.6 || at.y < area.top() + area.height() * 0.12 {
                self.scrolling = true;
            }
            if self.scrolling {
                steer.scroll = (at.y - want) * (1.0 - (-dt / 0.35).exp());
                self.scrolling = (at.y - want).abs() > 2.0;
            }
        }
        let settled = !self.follower.moving() && self.fade == if out { 0.0 } else { 1.0 } && steer.scroll == 0.0;
        ask_frame(ctx, &self.follower, settled, now, &params);
        self.steered |= steer.scroll != 0.0;
        steer
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn heard(loc: Loc, word: u32) -> Heard {
        Heard { loc, word, letter: 0, letters: 4, state: "Locked".into(), conf: 1.0, speed: 0.0, t: 0.0, slid: None }
    }

    /// Two lines of three words each, right to left (baselines 28 below each line's top, text size
    /// 20); words 100.. in the Quran.
    fn page() -> Vec<Slot> {
        let line = |y: f32, first: u16| (0..3).map(move |k| ((2, 7, first + k), Rect::from_min_size(pos2(200.0 - 60.0 * f32::from(k), y), vec2(50.0, 40.0)), y + 28.0, 20.0));
        line(0.0, 1).chain(line(40.0, 4)).collect()
    }

    #[test]
    fn positions_map_to_letters_right_to_left_from_the_word_heard() {
        let words = page();
        let h = heard((2, 7, 2), 101);
        // Word 101 is the second word: its box runs 140..190; a quarter in is 12.5 from the right.
        // Lifted a fifth of the text size (4) above its baseline (28).
        let (at, i) = point(101.25, &h, &words, 0.2).unwrap();
        assert_eq!((i, at.x, at.y), (1, 177.5, 24.0));
        // Two words on: the first word of the second line.
        assert_eq!(point(103.0, &h, &words, 0.2).unwrap().1, 3);
        assert!(point(99.0, &h, &words, 0.2).is_none(), "before the first word on screen");
        assert!(point(101.0, &heard((3, 1, 1), 500), &words, 0.2).is_none(), "heard word not on screen");
    }

    #[test]
    fn line_edges_cover_the_line_only() {
        let words = page();
        assert_eq!((line_of(&words, 1), line_of(&words, 5)), ((0, 2), (3, 5)));
        assert_eq!(edges(&words, line_of(&words, 1)), (80.0, 250.0));
        assert_eq!(edges(&words, line_of(&words, 5)), (80.0, 250.0));
    }

    #[test]
    fn a_fade_that_starts_after_the_app_slept_fades_over_frames() {
        // Parked in the ayah's last word with no news (a listener that sends only changes), the pointer
        // and the range settle and the app sleeps until the fade's delay is up; that frame must start
        // the fade, not finish it. Frames come when the last one asked, a little late, as in eframe.
        for delay in [1.0, 3.0, 5.0] {
            let p = Params { ayah_fade: Some(delay), ..Params::default() };
            let mut f = Follower::default();
            f.update(10.2, Reading::Locked, 0.9, 6.0, 0.0, 0.0, &p);
            f.set_ayah_end(Some(10.95));
            f.update(10.6, Reading::Paused, 0.9, 6.0, 0.3, 0.3, &p);
            let ctx = egui::Context::default();
            let (mut t, mut fade, mut fades) = (0.3, 1.0f32, Vec::new());
            while t < 0.3 + f64::from(delay) + 2.0 {
                let input = egui::RawInput { time: Some(t), ..Default::default() };
                let output = ctx.run_pass(input, |ctx| {
                    let dt = frame_dt(ctx);
                    f.tick(t, dt, &p);
                    let out = f.faded(t, &p);
                    fade = fade_step(fade, out, dt);
                    ask_frame(ctx, &f, !f.moving() && fade == if out { 0.0 } else { 1.0 }, t, &p);
                });
                fades.push((t, fade));
                t += output.viewport_output[&egui::ViewportId::ROOT].repaint_delay.as_secs_f64().max(1.0 / 60.0) + 0.0005;
            }
            let between = fades.iter().filter(|(_, a)| *a > 0.0 && *a < 1.0).count();
            assert!(between >= 12, "{delay} s: {between} frames part-faded: {fades:?}");
            assert_eq!(fades.last().unwrap().1, 0.0, "{delay} s: faded out");
        }
    }

    #[test]
    fn the_range_reaches_back_onto_the_line_before() {
        let words = page();
        let h = heard((2, 7, 2), 101);
        let place = |c: f32| point(c, &h, &words, 0.2);
        // The pointer half through word 104 (the second line's middle word, 140..190; y 64).
        let (at, i) = place(104.5).unwrap();
        assert_eq!((at, i), (pos2(165.0, 64.0), 4));
        let size = (30.0, 5.0, 16.0);
        // Both edges on its line: one rect, from the end's x to the start's (padded).
        let one = range_rects(&words, place, (104.0, 105.0), (at, i), size);
        assert_eq!(one, vec![Rect::from_min_max(pos2(125.0, 49.0), pos2(195.0, 79.0))]);
        // The start half through word 102 (the first line's last word, 80..130): this line from its
        // right end, and the line before from its left end to the start.
        let two = range_rects(&words, place, (102.5, 105.0), (at, i), size);
        assert_eq!(two, vec![Rect::from_min_max(pos2(125.0, 49.0), pos2(255.0, 79.0)), Rect::from_min_max(pos2(75.0, 9.0), pos2(110.0, 39.0))]);
        // Before the first word on screen: this line only, to its right end.
        assert_eq!(range_rects(&words, place, (99.5, 105.0), (at, i), size).len(), 1);
    }
}
