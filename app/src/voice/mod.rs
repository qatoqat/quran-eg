//! The voice pointer (native builds): quran-voice's listener hears the recitation and a dot on the
//! letter being read moves across the mushaf, inside an accuracy range (a rounded rectangle over the
//! words it could be on), as the quran.com extension does. `link` talks to the listener; `motion`
//! smooths its bursty updates into a glide at the reader's pace.

mod link;
mod motion;

use egui::{Color32, CornerRadius, Pos2, Rect, Stroke, StrokeKind, pos2, vec2};
use quran_core::settings::Voice as Settings;
use quran_core::settings::palette::parse_hex;
use quran_core::view::WordRef;

use self::link::{Event, Heard, Link};
use self::motion::{Follower, Jump, Params, Reading};
use crate::pointer::{Lens, ReadingPointer};
use crate::theme::Colors;

pub use self::link::Loc;
pub use self::motion::CURVES;

use crate::reader::Slot;

/// Seconds a new or jumping pointer takes to fade in.
const FADE: f32 = 0.25;

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
    last_frame: Option<f64>,
    /// Where the dot was drawn last, and its line's height (the glow's lens follows it).
    drawn: Option<(Pos2, f32)>,
    /// The words on screen as last sent to the listener, and when.
    sent: (Option<(Loc, Loc)>, f64),
    /// Since when the place heard (locked) has not been on screen.
    away_since: Option<f64>,
    scrolling: bool,
    /// The reader has just closed with ṣadaqa-llāhu-l-ʿaẓīm (until the next frame reports it).
    closed: bool,
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

/// Left and right edges of the line word `i` is on.
fn line_edges(words: &[Slot], i: usize) -> (f32, f32) {
    let y = words[i].1.center().y;
    let on_line = |j: usize| (words[j].1.center().y - y).abs() < words[i].1.height() * 0.5;
    let (mut a, mut b) = (i, i);
    while a > 0 && on_line(a - 1) {
        a -= 1;
    }
    while b + 1 < words.len() && on_line(b + 1) {
        b += 1;
    }
    let span = words[a..=b].iter().map(|(_, r, ..)| (r.left(), r.right()));
    span.fold((f32::INFINITY, f32::NEG_INFINITY), |(l, r), (a, b)| (l.min(a), r.max(b)))
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
            last_frame: None,
            drawn: None,
            sent: (None, 0.0),
            away_since: None,
            scrolling: false,
            closed: false,
        })
    }

    pub fn status(&self) -> &Status {
        &self.status
    }

    /// One line for the title bar: what the listener hears.
    pub fn summary(&self) -> String {
        match (&self.status, &self.last) {
            (Status::Starting, _) if self.link.started_it() => "◉ Starting the listener…".into(),
            (Status::Starting, _) => "◉ Joining the listener…".into(),
            (Status::Failed(e), _) => format!("Listening stopped: {e}"),
            (Status::Listening, None) => "◉ Listening: recite from any page".into(),
            (Status::Listening, Some(h)) if h.loc.0 == 0 => format!("◉ Listening · {} · {}", formula(h.loc.1), h.state),
            (Status::Listening, Some(h)) => format!("◉ Listening · {}:{} word {} · {}", h.loc.0, h.loc.1, h.loc.2, h.state),
        }
    }

    /// The glow's lens at the dot, if it glows.
    pub fn lens(&self, settings: &Settings) -> Option<Lens> {
        let (at, line_height) = self.drawn?;
        self.dot.lens_at(&settings.pointer, at, line_height)
    }

    fn heard(&mut self, h: Heard, now: f64, params: &Params) {
        self.closed |= closes(h.loc) && h.state == "Locked";
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

    /// The pointer appears afresh where it is going: it fades in with its trail gathered.
    fn respawn(&mut self) {
        self.fade = 0.0;
        self.line_y = None;
        self.dot.hide();
    }

    /// Reads the listener, moves and draws the pointer over `words` (the words on screen, in reading
    /// order) in `area`, and says how the reader should follow. Call after the reader has painted.
    pub fn frame(&mut self, ctx: &egui::Context, settings: &Settings, colors: &Colors, words: &[Slot], area: Rect) -> Steer {
        let now = ctx.input(|i| i.time);
        let dt = self.last_frame.map_or(1.0 / 60.0, |t| (now - t) as f32).clamp(0.0, 0.25);
        self.last_frame = Some(now);
        let params = Params::of(&settings.motion);
        for event in self.link.poll() {
            match event {
                Event::Connected => self.status = Status::Listening,
                Event::Heard(h) => self.heard(h, now, &params),
                Event::Failed(e) => self.status = Status::Failed(e),
            }
        }
        // The words on screen are where the listener looks first.
        if let (Some(a), Some(b)) = (words.first(), words.last())
            && self.sent.0 != Some((a.0, b.0))
            && now - self.sent.1 > 1.0
        {
            self.link.send_visible(a.0, b.0);
            self.sent = (Some((a.0, b.0)), now);
        }
        let mut steer = Steer { closed: std::mem::take(&mut self.closed), ..Steer::default() };
        self.drawn = None;
        // The pointer runs on no further than the end of the ayah being read (readers stop there).
        if let Some(h) = &self.last
            && let Ok(at) = words.binary_search_by(|(l, ..)| l.cmp(&h.loc))
        {
            let rest = words[at + 1..].iter().take_while(|(l, ..)| (l.0, l.1) == (h.loc.0, h.loc.1)).count();
            self.follower.set_end(Some(h.word as f32 + rest as f32 + 0.95));
        }
        let (Some(anchor), Some((x, sigma))) = (self.last.clone(), self.follower.tick(now, dt, &params)) else { return steer };
        let state = reading(&anchor.state);
        let lift = f32::from(settings.lift) / 10.0;
        let Some((at, i)) = point(x, &anchor, words, lift) else {
            // Locked on a place that is not on screen for a second: the reader should go there.
            let since = *self.away_since.get_or_insert(now);
            if state == Some(Reading::Locked) && anchor.conf > 0.8 && now - since > 1.0 && settings.auto_scroll {
                steer.go_to = Some(WordRef { chapter: anchor.loc.0, verse: anchor.loc.1, word: anchor.loc.2 });
                self.away_since = None;
            }
            self.dot.hide();
            return steer;
        };
        self.away_since = None;
        let (h, em) = (words[i].1.height(), words[i].3);
        if self.line_y.is_some_and(|y| (at.y - y).abs() > h * 0.6) {
            self.respawn();
        }
        self.line_y = Some(at.y);
        // Parked at the ayah's end (the reader pauses there): faded out until they go on.
        self.fade = if self.follower.parked() { (self.fade - dt / FADE).max(0.0) } else { (self.fade + dt / FADE).min(1.0) };
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Middle, egui::Id::unique("voice-range"))).with_clip_rect(area);
        let r = &settings.range;
        if r.enabled {
            // The words within the uncertainty, clipped to this line; reading runs right to left.
            let half = sigma * f32::from(r.spread) / 10.0;
            let (line_l, line_r) = line_edges(words, i);
            let edge = |c: f32| point(c, &anchor, words, lift).map(|(q, _)| q).filter(|q| (q.y - at.y).abs() < h * 0.5).map(|q| q.x);
            let right = edge(x - half).unwrap_or(line_r).min(line_r);
            let left = edge(x + half).unwrap_or(line_l).max(line_l);
            let pad = h * 0.12;
            let height = h * f32::from(r.size) / 10.0;
            let width = (right - left + 2.0 * pad).max(em * f32::from(settings.pointer.size) / 10.0);
            let rect = Rect::from_min_size(pos2(left - pad, at.y - height / 2.0), vec2(width, height));
            let opacity = f32::from(r.opacity) / 100.0;
            let radius = (height * f32::from(r.roundness) / 100.0).round().min(255.0) as u8;
            let outline = Stroke::new(f32::from(r.border), rgba(&r.color, (opacity * 2.5 + 0.25).min(1.0) * self.fade));
            painter.rect(rect, CornerRadius::same(radius), rgba(&r.color, opacity * self.fade), outline, StrokeKind::Inside);
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
        // Keep animating while the pointer glides, fades or scrolls; the listener's messages wake the
        // app otherwise.
        if self.fade < 1.0 || steer.scroll != 0.0 || (self.follower.pace() > 0.01 && state != Some(Reading::Paused)) {
            ctx.request_repaint();
        } else {
            ctx.request_repaint_after_secs(0.1);
        }
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
        assert_eq!(line_edges(&words, 1), (80.0, 250.0));
        assert_eq!(line_edges(&words, 5), (80.0, 250.0));
    }
}
