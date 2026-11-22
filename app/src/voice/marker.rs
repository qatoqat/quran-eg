//! The pointer over a recitation: what the listener heard (`heard`) moves a dot on the letter being read and an
//! accuracy range over the words it could be on (`frame`), and decides when the reader follows it elsewhere. The
//! live pointer (`Voice`, from quran-voice's listener) and the settings' preview (a scripted listener over a few
//! lines of its own) are both this, so what the preview shows is what the page does.

use std::collections::VecDeque;

use egui::epaint::RectShape;
use egui::{Color32, CornerRadius, LayerId, Pos2, Rect, Stroke, StrokeKind, Vec2, pos2, vec2};
use quran_core::settings::Voice as Settings;
use quran_core::settings::palette::parse_hex;
use quran_core::view::WordRef;

use super::Steer;
use super::follow::{Like, Offer, Pages, Stray, Verdict, near};
use super::hold::Hold;
use super::keep::{Keep, edges, line_of};
use super::lead::{self, Lead};
use super::link::{Heard, Loc};
use super::presence::Heard as Heard2;
use super::motion::{Drawn, Follower, Jump, Params, Plan, Reading};
use super::skipped::{Kind, Skipped, parting};
use crate::pointer::{Lens, ReadingPointer};
use crate::reader::Slot;
use crate::theme::Colors;

/// Seconds a new or jumping pointer takes to fade in.
const FADE: f32 = 0.25;
/// How far ahead (s) the next frame is looked for: from `FIRST`, twice as far each time, to `AHEAD` (no more than a
/// frame's step may be); a plan playing is looked at again `FLOWING` on at the latest (its clock lets go of held
/// letters and of the plan with nothing drawn moving).
const FIRST: f32 = 1.0 / 512.0;
const AHEAD: f32 = 0.25;
const FLOWING: f32 = 0.1;

/// Where and how a frame of the pointer is drawn.
pub struct Stage {
    /// Clock time (s) and seconds since the last frame.
    pub now: f64,
    pub dt: f32,
    /// Where it may draw (the words are inside).
    pub area: Rect,
    /// The layers the range and the dot are drawn on.
    pub layers: (LayerId, LayerId),
    /// What a point of the ring's line and of the range's outline is worth here: a miniature scales them as it
    /// scales the text, to keep the page's proportions.
    pub scale: f32,
    /// Scroll to keep the line being read in view.
    pub scrolls: bool,
}

/// When the pointer wants its next frame with no news from the listener.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub enum Wake {
    /// Something drawn changes by then.
    Now,
    /// Something drawn will have moved half a pixel by then, or a wait or a pause ends then (s).
    After(f32),
    #[default]
    Rest,
}

/// What a frame of the pointer did.
#[derive(Debug, Default)]
pub struct Shown {
    pub steer: Steer,
    /// Where the dot is (`None`: nothing heard on screen).
    pub at: Option<Pos2>,
    /// Parked at an ayah's end long enough to fade out.
    pub faded: bool,
    /// Back from a far place heard for a while: the slip.
    pub slip: Option<Stray>,
    /// A far place the reader may be taken to (the toast).
    pub offer: Option<Offer>,
    pub wake: Wake,
}

/// Seconds into a silence the reader keeps after which a report of another word is not followed (it heard a breath or
/// the room as the next word, or drifted on), and the listener's usual delay for a report without its evidence's age.
const SILENT: f32 = 0.6;
const LATENCY: f32 = 0.15;
/// Words a move on screen must go to be a jump (more than a correction), and seconds a place the reading jumped away
/// from stays one it left lately.
const LEAP: f32 = 20.0;
const LATELY: f64 = 30.0;

#[derive(Default)]
pub struct Marker {
    /// The audio time (s) the reader fell silent at, while they keep the silence.
    quiet_since: Option<f32>,
    /// A takbir led into this surah: the reader goes to its head (no place shows the takbir).
    takbir_to: Option<u16>,
    /// How far the page has faded out for a move (0: not at all): the pointer and the range fade with it.
    out: f32,
    follower: Follower,
    dot: ReadingPointer,
    last: Option<Heard>,
    /// Fade-in of a pointer that has just appeared or jumped, 0..=1.
    fade: f32,
    line_y: Option<f32>,
    /// Where the dot was drawn last, and its line's height (the glow's lens follows it).
    drawn: Option<(Pos2, f32)>,
    /// When to take the reader to a place heard off screen.
    pages: Pages,
    keep: Keep,
    /// A jump ahead waiting for the reading to go on there (tuned).
    hold: Hold,
    /// The place last drawn on screen (and its position: the index in the whole Quran plus the share read): what a far
    /// place may sound like. The places the reading jumped away from on screen lately, and when: a far look-alike near
    /// one of them is the listener coming back to the reader (it went on to a look-alike of where they were, and was
    /// followed there), not the reader sliding.
    seen: Option<(Loc, f32)>,
    jumped: VecDeque<(f32, f64)>,
    /// A basmala heard as the next surah's opening, and that surah, until the reader is taken there (once).
    lead: Lead,
    lead_to: Option<u16>,
    /// The word the dot was drawn on last, and where its box was then: how far the page has scrolled since.
    page_mark: Option<(Loc, Pos2)>,
    /// When the last frame was (the stage's clock), and whether anything drawn was on its way then; news has moved the
    /// follower's goals since, or its next wait or pause was to end by then (`Follower::wake`).
    ticked: Option<(f64, bool)>,
    news: bool,
    due: Option<f64>,
    /// The scroll the last frame asked of the reader.
    asked: f32,
    /// The words the reading went past unread.
    skipped: Skipped,
}

/// What a frame draws: the dot, the range's rectangles, and how opaque both are (`shade`).
#[derive(Debug, Clone, PartialEq)]
struct Look {
    at: Pos2,
    rects: Vec<Rect>,
    shade: f32,
}

impl Look {
    /// How far `then` is from this in steps that show: half a pixel (`px`) of the dot or of a corner of the range, or a
    /// 1/255 step of the most opaque part drawn (`alpha`, its opacity in full shade); nothing drawn, or a range of
    /// another shape, at once.
    fn change(&self, then: Option<&Look>, (px, alpha): (f32, f32)) -> f32 {
        let unseen = |l: &Look| l.shade * alpha * 255.0 < 0.5;
        if unseen(self) && then.is_none_or(unseen) {
            return 0.0;
        }
        let Some(then) = then.filter(|t| t.rects.len() == self.rects.len()) else { return f32::INFINITY };
        let moved = |a: Pos2, b: Pos2| (a - b).abs().max_elem();
        let corners = self.rects.iter().zip(&then.rects).map(|(a, b)| moved(a.min, b.min).max(moved(a.max, b.max)));
        let most = corners.fold(moved(self.at, then.at), f32::max);
        (most / px).max((self.shade - then.shade).abs() * alpha * 255.0)
    }
}

/// `#rrggbb` at an opacity.
fn rgba(hex: &str, alpha: f32) -> Color32 {
    let [r, g, b] = parse_hex(hex).unwrap_or([255, 140, 0]);
    Color32::from_rgba_unmultiplied(r, g, b, (alpha.clamp(0.0, 1.0) * 255.0).round() as u8)
}

/// How rounded rectangles over words look: the accuracy range's, or the skipped words'.
pub struct Boxes<'a> {
    pub color: &'a str,
    /// Percent.
    pub opacity: u8,
    /// Corner radius in percent of a rectangle's height, outline width in points, edge blur 0..=10.
    pub roundness: u8,
    pub border: u8,
    pub blur: u8,
}

impl<'a> From<&'a quran_core::settings::VoiceRange> for Boxes<'a> {
    fn from(r: &'a quran_core::settings::VoiceRange) -> Self {
        Boxes { color: &r.color, opacity: r.opacity, roundness: r.roundness, border: r.border, blur: r.blur }
    }
}

impl<'a> From<&'a quran_core::settings::VoiceSkipped> for Boxes<'a> {
    fn from(s: &'a quran_core::settings::VoiceSkipped) -> Self {
        Boxes { color: &s.color, opacity: s.opacity, roundness: s.roundness, border: s.border, blur: s.blur }
    }
}

impl<'a> From<&'a quran_core::settings::VoiceSimilar> for Boxes<'a> {
    fn from(s: &'a quran_core::settings::VoiceSimilar) -> Self {
        Boxes { color: &s.color, opacity: s.opacity, roundness: s.roundness, border: s.border, blur: s.blur }
    }
}

/// Draws `rects` as the range is drawn, at `shade` of the look's opacity: filled, outlined (a point of it worth `scale`;
/// an outline stays an outline: at most a third of the height, the fill keeps the rest), the edges blurred (10 softens
/// over half the height: epaint's feathering, as for shadows).
pub fn paint_boxes(painter: &egui::Painter, rects: &[Rect], look: Boxes, shade: f32, scale: f32) {
    let opacity = f32::from(look.opacity) / 100.0;
    for &rect in rects {
        let height = rect.height();
        let radius = CornerRadius::same((height * f32::from(look.roundness) / 100.0).round().min(255.0) as u8);
        let outline = Stroke::new((f32::from(look.border) * scale).min(height / 3.0), rgba(look.color, (opacity * 2.5 + 0.25).min(1.0) * shade));
        let soft = height * f32::from(look.blur) * 0.05;
        painter.add(RectShape::filled(rect, radius, rgba(look.color, opacity * shade)).with_blur_width(soft));
        if outline.width > 0.0 {
            // Blurred no wider than itself, it keeps its width at any edge blur: with the fill's blur a thin outline would
            // spread faintly over all of it, beyond the words.
            painter.add(RectShape::stroke(rect, radius, outline, StrokeKind::Inside).with_blur_width(soft.min(outline.width)));
        }
    }
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

/// Where a message puts the reader: the index of the word in the whole Quran plus the share of it read (the
/// middle of the letter heard, short of the word's end).
pub fn position(h: &Heard) -> f32 {
    let frac = if h.letters > 0 { ((f32::from(h.letter) + 0.5) / f32::from(h.letters)).min(0.95) } else { 0.5 };
    h.word as f32 + frac
}

/// The point at word position `c` (the index of a word in the whole Quran plus the share of it read,
/// right to left), `lift` (a share of the text size) above the baseline: the word heard last anchors
/// the index among the words on screen, which are in reading order. Also returns the word's index in
/// `words`.
pub fn point(c: f32, anchor: &Heard, words: &[Slot], lift: f32) -> Option<(Pos2, usize)> {
    let at = words.binary_search_by(|(l, ..)| l.cmp(&anchor.loc)).ok()?;
    let i = usize::try_from(at as i64 + c.floor() as i64 - i64::from(anchor.word)).ok()?;
    let (_, rect, baseline, em) = words.get(i)?;
    let f = (c - c.floor()).clamp(0.0, 1.0);
    Some((pos2(rect.right() - f * rect.width(), baseline - lift * em), i))
}

/// Just inside the last word of the line word `ev` (the index in the whole Quran) is on, by the words on screen and the
/// word heard last, which anchors the index among them; none off screen.
fn line_stop(words: &[Slot], anchor: &Heard, ev: u32) -> Option<f32> {
    let at = words.binary_search_by(|(l, ..)| l.cmp(&anchor.loc)).ok()?;
    let i = usize::try_from(at as i64 + i64::from(ev) - i64::from(anchor.word)).ok().filter(|&i| i < words.len())?;
    let (_, last) = line_of(words, i);
    Some((i64::from(anchor.word) + last as i64 - at as i64) as f32 + 0.95)
}

/// Where the accuracy range is drawn, its edges at word positions `start` and `end` and the pointer at
/// `at` on word `i`: on the pointer's line from `end` to `start` (to the line's end where an edge lies
/// off it; reading runs right to left), and, when `start` lies on the line before (the range reaching
/// back to a reader heard there while the pointer waits) in the same surah's ayahs, there too, from that
/// line's left end to `start` (never over the Bismillah line before a surah's first ayah). `place` finds
/// a position's point and word; `size` is the height, the padding beyond the words and the least width.
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
        && words[j].0.0 == words[i].0.0
        && words[j].0.1 > 0
    {
        let (before_left, _) = edges(words, line_of(words, j));
        rects.push(Rect::from_min_max(pos2(before_left - pad, q.y - height / 2.0), pos2(q.x + pad, q.y + height / 2.0)));
    }
    rects
}

/// How a far place heard (`h`) sounds beside the ayah last followed on screen (`seen`). Alike when the listener slid
/// there from it or weighed one against the other, or went on there at the very word the reader had come to (look-alike
/// ayahs share their words up to where they part, as 62:7 and 2:95 do), or when it is al-Fatihah after a basmala that
/// opened another surah (the listener chaining the basmala as al-Fatihah's first ayah); the reader is not taken there by
/// itself. Another ayah alike, from an ayah, and not near a place the reading jumped away from (`jumped`): where the
/// reader may have slid into from it (`slides`: when such slides are marked).
fn alike(h: &Heard, seen: Option<Loc>, jumped: &VecDeque<(f32, f64)>, slides: bool) -> Like {
    let Some((s, a, w)) = seen else { return Like::Not };
    let ayah = |l: &Loc| (l.0, l.1);
    let sounds = h.slid.is_some_and(|(_, to)| ayah(&to) == ayah(&h.loc))
        || h.alts.iter().any(|l| ayah(l) == (s, a))
        || (h.loc.2 >= 3 && (w..=w + 2).contains(&h.loc.2))
        || (a == 0 && h.loc.0 == 1);
    let other = s > 0 && a > 0 && h.loc.1 > 0 && ayah(&h.loc) != (s, a) && !jumped.iter().any(|j| near(h.word, j.0 as u32));
    match (sounds, slides && other) {
        (false, _) => Like::Not,
        (true, false) => Like::Sounds,
        (true, true) => Like::Slide,
    }
}

/// egui's steady measure of a frame, which takes the first frame after the app slept (nothing moving, no news) for
/// one frame, not the whole sleep: the step the marker takes after a rest (`Stage::dt`). While anything moves it steps
/// by the time since its last frame instead, however far apart the frames it asks for are.
pub fn frame_dt(ctx: &egui::Context) -> f32 {
    ctx.input(|i| i.stable_dt).min(0.25)
}

/// The pointer's fade `dt` on: out (parked at an ayah's end) or in.
fn fade_step(fade: f32, out: bool, dt: f32) -> f32 {
    if out { (fade - dt / FADE).max(0.0) } else { (fade + dt / FADE).min(1.0) }
}

/// The most opaque part of the pointer drawn, in full shade: what a step of its shade shows as.
fn opacity(settings: &Settings) -> f32 {
    let r = &settings.range;
    let range = f32::from(r.opacity) / 100.0;
    let range = if r.border > 0 { (range * 2.5 + 0.25).min(1.0) } else { range };
    let dot = f32::from(settings.pointer.opacity.clamp(5, 100)) / 100.0;
    f32::max(if r.enabled { range } else { 0.0 }, if settings.pointer.enabled { dot } else { 0.0 })
}

impl Marker {
    /// What the listener heard last.
    pub fn last(&self) -> Option<&Heard> {
        self.last.as_ref()
    }

    /// The far place heard while the recitation is lost for now.
    pub fn stray(&self) -> Option<&Stray> {
        self.pages.stray()
    }

    pub fn set_out(&mut self, out: f32) {
        self.out = out;
    }

    /// The ayah last followed on screen.
    pub fn seen(&self) -> Option<Loc> {
        self.seen.map(|s| s.0)
    }

    /// The reader moved the view themselves, now showing `words`: places off screen are far from it until the
    /// recitation is followed there, and (scrolling continuously) the view is left where they put it until the next line.
    pub fn reader_moved(&mut self, words: &[Slot]) {
        let ayah = |w: &&Slot| w.0.1 > 0;
        if let Some(first) = words.iter().find(ayah) {
            self.pages.chosen(first.0);
        }
        let on = self.seen.and_then(|(loc, _)| words.binary_search_by(|(l, ..)| l.cmp(&loc)).ok());
        self.keep.reader_moved(words, on);
        self.skipped.moved(words);
    }

    /// The words the reading went past unread.
    pub fn skipped(&mut self) -> &mut Skipped {
        &mut self.skipped
    }

    /// The reader chose in the toast: to go to the far place now, or to stay.
    pub fn choose(&mut self, go: bool) -> Option<WordRef> {
        if !go {
            self.pages.decline();
            return None;
        }
        let to = self.pages.stray()?.to;
        self.pages.go();
        Some(WordRef { chapter: to.0, verse: to.1, word: to.2 })
    }

    /// The glow's lens at the dot, if it glows.
    pub fn lens(&self, settings: &Settings) -> Option<Lens> {
        let (at, line_height) = self.drawn?;
        self.dot.lens_at(&settings.pointer, at, line_height)
    }

    /// Takes in a message of the listener at clock time `now`; says how the pointer goes there, if it shows.
    pub fn heard(&mut self, h: Heard, now: f64, params: &Params) -> Option<Jump> {
        // Held until the voice is back: in a silence the pointer stays where the reading stopped.
        if self.heard_in_silence(&h) {
            return None;
        }
        let (h, leads) = self.lead.heard(h, now);
        self.takbir_to = self.lead.takbir().or(self.takbir_to);
        if leads {
            // On another line, and its words' indices are the last ayah's: the pointer appears there afresh.
            self.follower.hide();
            self.lead_to = Some(h.loc.0);
            // (Its first word: after a Bismillah line's four, or al-Fatihah's own first.)
            let first = if h.loc.1 == 0 { h.word + 5 - u32::from(h.loc.2) } else { h.word + 1 - u32::from(h.loc.2) };
            self.pages.arrive(first, (h.loc.0, 1, 1));
        } else if params.tuned && h.loc.0 > 0 && reading(&h.state).is_some() && !self.hold.take(position(&h), self.follower.target(), now) {
            // A jump ahead, held: the pointer stays, and the place it shows stays the one heard before (its word was
            // heard, though: not a word gone past unread).
            self.skipped.held_back(&h, now);
            return None;
        }
        // (A jump the hold lets go arrived where it was first heard; the pace is the one the reading had come at.)
        let (held, pace) = (self.hold.confirmed(), self.follower.pace());
        if !matches!(reading(&h.state), Some(Reading::Locked | Reading::Drifting | Reading::Paused)) {
            // Searching, or unsure: no longer followed well where it was.
            self.pages.lost();
        }
        let jump = match reading(&h.state).filter(|_| h.loc.0 > 0) {
            None => {
                self.follower.hide();
                None
            }
            Some(r) => {
                // With tajweed timing, a Locked place's plan is what the pointer plays.
                let jump = match h.plan.as_ref().filter(|_| params.plan && r == Reading::Locked) {
                    Some(plan) => self.follower.update_plan(plan, r, h.conf, h.speed, h.t, now, params),
                    None => self.follower.update(position(&h), r, h.conf, h.speed, h.t, now, params),
                };
                if jump == Jump::Respawn {
                    self.respawn();
                }
                Some(jump)
            }
        };
        if let Some(r) = reading(&h.state) {
            self.skipped.heard(&h, r, held, pace, params.skipped, now);
        }
        self.last = Some(h);
        self.news();
        jump
    }

    /// News of the reader moved the follower's goals, or of the voice its run-on: the next frame steps no more than a
    /// frame's time (`step`).
    fn news(&mut self) {
        self.news = true;
    }

    /// A new plan while the place shown stays (the reader got further within it), `t` s of audio heard.
    pub fn replan(&mut self, plan: &Plan, t: f32, now: f64, params: &Params) {
        // (A plan on a Bismillah line would be in the listener's indices of the basmala; one while a jump is held is
        // the held place's, which the pointer does not go to before the hold lets it.)
        let Some(h) = self.last.as_ref().filter(|h| params.plan && h.loc.0 > 0 && h.loc.1 > 0 && !(params.tuned && self.hold.holding())) else { return };
        if let Some(r) = reading(&h.state)
            && self.follower.update_plan(plan, r, h.conf, h.speed, t, now, params) == Jump::Respawn
        {
            self.respawn();
        }
        self.news();
    }

    /// The reader's voice came or went, or a silence began or ended (the link's presence), at clock time `now`.
    /// The link's news of the voice at clock time `now`: present or not, in a silence or not, and the audio time of its
    /// last loud level (none in the preview, whose reports have no audio time).
    pub fn voice(&mut self, heard: Heard2, loud: Option<f32>, now: f64) {
        self.follower.voice(heard.present, heard.quiet, now);
        self.quiet_since = loud.filter(|_| heard.quiet);
        self.news();
    }

    /// Whether a report comes from a silence the reader keeps: another word than the pointer's, its evidence over
    /// `SILENT` s into the silence. The words before it, heard late, do not.
    fn heard_in_silence(&self, h: &Heard) -> bool {
        let heard_at = h.t - h.plan.as_ref().map_or(LATENCY, |p| p.age);
        match (self.quiet_since, self.follower.target()) {
            (Some(quiet), Some(at)) => heard_at > quiet + SILENT && (position(h) - at).abs() >= 0.6,
            _ => false,
        }
    }

    /// The words were laid out again under the pointer (the preview's passage: another size, font or window): it
    /// shows at its word's new place as it was, with nothing behind it to smear and no line change to fade over.
    pub fn relaid(&mut self) {
        self.line_y = None;
        self.page_mark = None;
        self.dot.hide();
    }

    /// The reader's pace as the pointer runs on with no news, words a second.
    pub fn pace(&self) -> f32 {
        self.follower.pace()
    }

    /// The pointer appears afresh where it is going: it fades in, with nothing behind it to smear.
    fn respawn(&mut self) {
        self.fade = 0.0;
        self.line_y = None;
        self.dot.hide();
    }

    /// Seconds since the last frame for what moves, at a frame at `now`: the time since it while anything drawn was on
    /// its way then, else `frame` (a frame's: a wait or a pause that ends after a sleep starts its glide or fade then,
    /// instead of finishing it in one step). After news, or with a wait or a pause over since, no more than a frame's,
    /// as at steady frames: the time since a frame far apart (the old goals' slow last bit, under half a pixel) is not
    /// to be taken toward the new ones. At most a quarter of a second.
    fn step(&self, now: f64, frame: f32) -> f32 {
        let changed = self.news || self.due.is_some_and(|due| due <= now + 1e-6);
        let dt = match self.ticked {
            Some((at, true)) if changed => ((now - at) as f32).min(frame),
            Some((at, true)) => (now - at) as f32,
            _ => frame,
        };
        dt.clamp(0.0, 0.25)
    }

    /// Seconds until what is drawn (`seen`) would look different (`Look::change`, in `steps`) with no news, as `look`
    /// draws a place at a shade: the follower and the fade looked at ahead, from `FIRST` on and twice as far each time,
    /// to `AHEAD`. None within it.
    fn ahead(&self, now: f64, params: &Params, seen: &Look, look: impl Fn(Drawn, f32) -> Option<Look>, steps: (f32, f32)) -> Option<f32> {
        let (mut follower, mut fade) = (self.follower.clone(), self.fade);
        let (mut t, mut was, mut h) = (0.0, 0.0, FIRST);
        while h <= AHEAD {
            let at = now + f64::from(h);
            let then = follower.tick(at, h - t, params).and_then(|d| {
                fade = fade_step(fade, follower.faded(at, params), h - t);
                look(d, fade * follower.dim(at) * (1.0 - self.out))
            });
            let change = seen.change(then.as_ref(), steps);
            if change >= 1.0 {
                // As far between the last look and this one as a smooth change is; a jump (the dot over the space
                // between two words, a fade over, gone) is drawn at this look.
                return Some(if change > 4.0 { h } else { t + (h - t) * (1.0 - was) / (change - was) });
            }
            (t, was, h) = (h, change, h * 2.0);
        }
        None
    }

    /// Moves and draws the pointer over `words` (the words in the stage's area, in reading order), and says how
    /// the reader should follow. Call after the words are painted.
    pub fn frame(&mut self, ctx: &egui::Context, stage: &Stage, settings: &Settings, colors: &Colors, words: &[Slot]) -> Shown {
        let Stage { now, area, .. } = *stage;
        let dt = self.step(now, stage.dt);
        // (At rest, unless something drawn is found on its way below.)
        (self.ticked, self.news, self.due) = (Some((now, false)), false, None);
        self.lead.view(words, area, ctx.input(|i| i.pointer.hover_pos()));
        // The words gone past unread (and those a slide into a look-alike left), under the range (painted first on its
        // layer) wherever the pointer is; they change only with news, and ask for no frames.
        let lift = f32::from(settings.lift) / 10.0;
        for (kind, on) in [(Kind::Skipped, settings.skipped.enabled), (Kind::Similar, settings.similar.enabled)] {
            if !on {
                self.skipped.dismiss(kind);
            }
        }
        if !self.skipped.is_empty() {
            let painter = ctx.layer_painter(stage.layers.0).with_clip_rect(area);
            let (skipped, similar) = (&settings.skipped, &settings.similar);
            for (kind, size, look) in [(Kind::Skipped, skipped.size, Boxes::from(skipped)), (Kind::Similar, similar.size, Boxes::from(similar))] {
                paint_boxes(&painter, &self.skipped.rects(kind, words, lift, size), look, 1.0 - self.out, stage.scale);
            }
        }
        let params = Params::of(settings);
        let far_wait = f64::from(settings.far_wait_ms) / 1000.0;
        let mut shown = Shown { offer: self.pages.offer(now, far_wait).filter(|_| settings.auto_scroll), ..Shown::default() };
        shown.steer.lead = self.takbir_to.take().filter(|_| settings.auto_scroll);
        self.drawn = None;
        // The pointer runs on no further than the end of the ayah being read (readers stop there).
        if let Some(h) = &self.last
            && let Ok(at) = words.binary_search_by(|(l, ..)| l.cmp(&h.loc))
        {
            let rest = words[at + 1..].iter().take_while(|(l, ..)| (l.0, l.1) == (h.loc.0, h.loc.1)).count();
            self.follower.set_ayah_end(Some(h.word as f32 + rest as f32 + 0.95));
        }
        // With tajweed timing, nor past the end of the line the evidence is on: the pointer changes line (and fades over,
        // as ever) only once the reader is heard on the next.
        let stop = self.last.as_ref().filter(|_| params.plan && params.line_stop).and_then(|h| line_stop(words, h, self.follower.evidence()?));
        self.follower.set_stop(stop);
        let (Some(anchor), Some(drawn)) = (self.last.clone(), self.follower.tick(now, dt, &params)) else { return shown };
        let state = reading(&anchor.state);
        let sure = state == Some(Reading::Locked) && anchor.conf > 0.8;
        let place = |c: f32| lead::point(c, &anchor, words, lift);
        let Some((at, i)) = place(drawn.x) else {
            // Not on screen: the reader goes there after a second, or, far from where the recitation
            // was being followed, once it has kept being followed there (lost until then); to the start of
            // the surah a basmala leads into at once. Hidden meanwhile, the pointer fades in wherever it
            // shows again.
            if anchor.loc.1 == 0 {
                shown.steer.lead = self.lead_to.take().filter(|_| settings.auto_scroll);
            } else {
                let like = alike(&anchor, self.seen.map(|s| s.0), &self.jumped, settings.similar.enabled);
                if self.pages.off_screen(anchor.word, anchor.loc, sure, now, far_wait, like) == Verdict::Go && settings.auto_scroll {
                    shown.steer.go_to = Some(WordRef { chapter: anchor.loc.0, verse: anchor.loc.1, word: anchor.loc.2 });
                }
                // Gone on into a look-alike of the ayah being read: the words that ayah goes on with are marked.
                if let (Some(slide), Some(seen)) = (self.pages.slid(now), self.seen)
                    && let Some(from) = parting(anchor.slid, anchor.loc, seen)
                {
                    self.skipped.slid(from, slide.to);
                }
            }
            shown.offer = self.pages.offer(now, far_wait).filter(|_| settings.auto_scroll && shown.steer.go_to.is_none());
            if let Some(after) = self.pages.wake(now) {
                shown.wake = Wake::After(after);
            }
            self.fade = 0.0;
            self.page_mark = None;
            self.dot.hide();
            return shown;
        };
        // The page scrolled under the pointer since the last frame: its line and its smudge are on the page, and move
        // with it (a scroll is no line change, and leaves no streak).
        let scrolled = self.page_mark.and_then(|(loc, was)| Some(words[words.binary_search_by(|(l, ..)| l.cmp(&loc)).ok()?].1.min - was));
        if let Some(by) = scrolled.filter(|by| *by != Vec2::ZERO) {
            self.dot.scrolled(by);
            self.line_y = self.line_y.map(|y| y + by.y);
        }
        // Half a pixel: what a frame must have moved by to be worth drawing. The page not scrolled as the last frame
        // asked (by half of it or more): the view is at an end of what it shows.
        let px = 0.5 / ctx.pixels_per_point();
        let asked = std::mem::take(&mut self.asked);
        if asked.abs() >= px && scrolled.is_some_and(|by| (by.y + asked).abs() > 0.5 * asked.abs()) {
            self.keep.stuck(words, i);
        }
        self.page_mark = Some((words[i].0, words[i].1.min));
        if anchor.loc.1 == 0 {
            // The surah a basmala leads into is on screen already.
            self.lead_to = None;
        }
        let here = position(&anchor);
        self.jumped.retain(|j| now - j.1 <= LATELY);
        if let Some((_, was)) = self.seen.filter(|(_, was)| (here - was).abs() > LEAP) {
            self.jumped.push_back((was, now));
        }
        self.seen = Some((anchor.loc, here));
        // Back from a far place: a slip, or the listener's mistake (where a mistake notifier would say so).
        shown.slip = self.pages.on_screen(anchor.word, anchor.loc, sure, now);
        let (h, em) = (words[i].1.height(), words[i].3);
        if self.line_y.is_some_and(|y| (at.y - y).abs() > h * 0.6) {
            self.respawn();
        }
        self.line_y = Some(at.y);
        // Parked at the ayah's end, the reader pausing there for the delay: faded out until they go on; dimmed in a
        // silence inside an ayah.
        let out = self.follower.faded(now, &params);
        self.fade = fade_step(self.fade, out, dt);
        let shade = self.fade * self.follower.dim(now) * (1.0 - self.out);
        let r = &settings.range;
        // The range's height, padding beyond the words and least width on word `i`'s line, and what a frame draws.
        let size = |i: usize| {
            let (h, em): (f32, f32) = (words[i].1.height(), words[i].3);
            (h * f32::from(r.size) / 10.0, h * 0.12, em * f32::from(settings.pointer.size) / 10.0)
        };
        let look = |d: Drawn, shade: f32| {
            let (at, i) = place(d.x)?;
            let rects = if r.enabled { range_rects(words, place, (d.start, d.end), (at, i), size(i)) } else { Vec::new() };
            Some(Look { at, rects, shade })
        };
        let seen = Look { at, rects: look(drawn, shade).map_or_else(Vec::new, |l| l.rects), shade };
        if r.enabled {
            let painter = ctx.layer_painter(stage.layers.0).with_clip_rect(area);
            paint_boxes(&painter, &seen.rects, Boxes::from(r), shade, stage.scale);
        }
        let mut smudging = false;
        if settings.pointer.enabled {
            let ring = f32::from(settings.ring) * stage.scale;
            // ("Shrink with speed" is the mouse pointer's: at a reading pace this one would shrink by a hundredth.)
            let look = (settings.pointer.shrink > 0).then(|| quran_core::settings::Pointer { shrink: 0, ..settings.pointer.clone() });
            smudging = self.dot.show_at(ctx, stage.layers.1, look.as_ref().unwrap_or(&settings.pointer), colors, area, em, Some(ring), at, shade);
            self.drawn = Some((at, em));
        }
        let mut scroll = None;
        if stage.scrolls && matches!(state, Some(Reading::Locked | Reading::Paused | Reading::Drifting)) {
            shown.steer.scroll = self.keep.scroll(words, i, drawn.x.fract(), at.y, area, dt, self.follower.pace(), params.continuous, px);
            (scroll, self.asked) = (self.keep.wake(), shown.steer.scroll);
        }
        // The next frame: once anything drawn would have moved half a pixel or changed by a step of opacity, the scroll
        // too; a glide closer than half a pixel to its goal is put there (the rest of it would never show).
        let next = self.ahead(now, &params, &seen, look, (px, opacity(settings)));
        let width = |c: f32| place(c).map_or(words[i].1.width(), |(_, j)| words[j].1.width()).max(1.0);
        let rest = self.follower.settle([px / width(drawn.x), px / width(drawn.start), px / width(drawn.end)], now, &params);
        let rest = [rest[0], rest[1] || !r.enabled, rest[2] || !r.enabled];
        // A smudge drawing back into a head that has come to rest: at every frame until it has. A glide not at rest yet
        // looked at again within the look ahead's reach; a plan's clock within a tenth of a second.
        let smudge = (smudging && rest[0]).then_some(FIRST);
        let gliding = (rest != [true; 3]).then_some(AHEAD);
        let flowing = self.follower.flowing().then_some(FLOWING);
        let soon = [next, scroll, smudge, gliding, flowing].into_iter().flatten().reduce(f32::min);
        self.ticked = Some((now, soon.is_some()));
        // A wait or a pause that is to end asks for a frame then; the listener's news wakes the app otherwise.
        let timer = self.follower.wake(now, &params);
        self.due = timer.map(|secs| now + f64::from(secs));
        shown.wake = match [soon, timer].into_iter().flatten().reduce(f32::min) {
            Some(secs) if secs <= FIRST => Wake::Now,
            Some(secs) => Wake::After(secs),
            None => Wake::Rest,
        };
        shown.at = Some(at);
        shown.faded = out;
        shown
    }
}

#[cfg(test)]
mod tests {
    use super::super::ask_frame;
    use super::*;

    fn heard(loc: Loc, word: u32) -> Heard {
        Heard { loc, word, letter: 0, letters: 4, state: "Locked".into(), conf: 1.0, speed: 0.0, t: 0.0, slid: None, alts: Vec::new(), plan: None }
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

    /// A frame of `m` at `now` over `words` in a 300 × `height` area, as the page draws it (a step of `frame` after a rest
    /// or news; none: egui's steady measure); what it showed, and how long egui would wait for the next frame.
    fn shown_at(ctx: &egui::Context, m: &mut Marker, settings: &Settings, words: &[Slot], (now, frame): (f64, Option<f32>), height: f32) -> (Shown, f64) {
        let colors = crate::theme::Colors::of(&quran_core::settings::Settings::default(), false);
        let mut shown = None;
        let mut output = ctx.run_pass(egui::RawInput { time: Some(now), ..Default::default() }, |ctx| {
            let layer = |n| LayerId::new(egui::Order::Middle, egui::Id::unique(n));
            let area = Rect::from_min_size(pos2(0.0, 0.0), vec2(300.0, height));
            let stage = Stage { now, dt: frame.unwrap_or_else(|| frame_dt(ctx)), area, layers: (layer("r"), layer("p")), scale: 1.0, scrolls: true };
            let s = m.frame(ctx, &stage, settings, &colors, words);
            ask_frame(ctx, s.wake);
            shown = Some(s);
        });
        output.textures_delta.clear();
        (shown.unwrap(), output.viewport_output[&egui::ViewportId::ROOT].repaint_delay.as_secs_f64())
    }

    #[test]
    fn a_fade_that_starts_after_the_app_slept_fades_over_frames() {
        // Parked in the ayah's last word with no news (a listener that sends only changes), the pointer and the range
        // settle and the app sleeps until the fade's delay is up; that frame must start the fade, not finish it. Frames
        // come when the last one asked (or the news), a little late and at most 60 a second, as in eframe.
        let words = page();
        for delay in [1000, 3000, 5000] {
            let mut settings = Settings::default();
            settings.motion.ayah_fade_delay_ms = delay;
            let (params, ctx, mut m) = (Params::of(&settings), egui::Context::default(), Marker::default());
            m.heard(Heard { letter: 1, ..heard((2, 7, 6), 105) }, 0.0, &params);
            let (mut t, mut fades, mut paused, mut slept) = (0.0, Vec::new(), false, 0.0f64);
            while t < 0.3 + f64::from(delay) / 1000.0 + 2.0 {
                if !paused && t >= 0.3 {
                    m.heard(Heard { letter: 2, state: "Paused".into(), ..heard((2, 7, 6), 105) }, t, &params);
                    paused = true;
                }
                let (_, wait) = shown_at(&ctx, &mut m, &settings, &words, (t, None), 400.0);
                fades.push((t, m.fade));
                if t < 0.3 + f64::from(delay) / 1000.0 - 0.01 {
                    slept = slept.max(wait.min(9.0));
                }
                let next = t + wait.max(1.0 / 60.0) + 0.0005;
                t = if paused { next } else { next.min(0.3) };
            }
            let between = fades.iter().filter(|(_, a)| *a > 0.0 && *a < 1.0).count();
            assert!(between >= 12, "{delay} ms: {between} frames part-faded: {fades:?}");
            assert_eq!(fades.last().unwrap().1, 0.0, "{delay} ms: faded out");
            // Its glide over (in under two seconds), it slept until the fade.
            assert!(delay < 3000 || slept > 0.5, "{delay} ms: it slept before the fade ({slept} s)");
        }
    }

    #[test]
    fn a_view_that_cannot_scroll_further_is_left_as_it_is() {
        // The line read low in a short view that is at its end (the words never move however it is asked to scroll):
        // one frame asks, the next sees the page did not move, and the pointer comes to rest there.
        let (words, settings) = (page(), Settings::default());
        let (ctx, mut m) = (egui::Context::default(), Marker::default());
        m.heard(heard((2, 7, 5), 104), 0.0, &Params::of(&settings));
        let (mut t, mut asked) = (0.0, Vec::new());
        while t < 3.0 {
            let (shown, wait) = shown_at(&ctx, &mut m, &settings, &words, (t, None), 100.0);
            asked.push((t, shown.steer.scroll, shown.wake));
            t += wait.max(1.0 / 60.0);
        }
        assert!(asked[0].1 > 0.0 && asked[2..].iter().all(|a| a.1 == 0.0), "{asked:?}");
        assert!(asked.last().is_some_and(|a| a.2 == Wake::Rest) && asked.len() < 40, "{} frames: {asked:?}", asked.len());
        // The reader moving the view lets it scroll again.
        m.reader_moved(&words);
        assert!(shown_at(&ctx, &mut m, &settings, &words, (3.1, None), 100.0).0.steer.scroll > 0.0);
    }

    /// A frame: when, where the dot was drawn and how faded, and when it asked for the next.
    type Frame = (f64, Option<(Pos2, f32)>, Wake);

    #[test]
    fn frames_come_as_the_pointer_moves_half_a_pixel_and_stop_once_it_rests() {
        // Reading three lines at the user's pace (a report every 0.32 s, two a word), then silence. Frames drawn when
        // the marker asks show the pointer where steady fine frames do at the same moments, never let it fall more than
        // a pixel behind them in between, and stop soon after the reader does.
        let words: Vec<Slot> = (0..12u16)
            .map(|k| {
                let rect = Rect::from_min_size(pos2(250.0 - 80.0 * f32::from(k % 3), 80.0 + 40.0 * f32::from(k / 3)), vec2(70.0, 40.0));
                ((2, 7, k + 1), rect, rect.top() + 28.0, 20.0)
            })
            .collect();
        let settings = Settings::default();
        let params = Params::of(&settings);
        let news: Vec<(f64, Heard)> = (0..14u16).map(|k| (0.1 + 0.32 * f64::from(k), Heard { letter: (k % 2) * 2, ..heard((2, 7, k / 2 + 1), 100 + u32::from(k / 2)) })).collect();
        // Draws frames from news to news, a frame `next` after each (from its wait) until `until`.
        let run = |next: &dyn Fn(f64) -> f64, until: f64| {
            let (ctx, mut m) = (egui::Context::default(), Marker::default());
            let (mut t, mut k, mut frames) = (0.1, 0, Vec::<Frame>::new());
            while t < until {
                while k < news.len() && news[k].0 <= t + 1e-9 {
                    m.heard(news[k].1.clone(), news[k].0, &params);
                    k += 1;
                }
                // (Both take the fine frames' step after news and rests: they differ only by the frames between.)
                let (shown, wait) = shown_at(&ctx, &mut m, &settings, &words, (t, Some(1.0 / 480.0)), 400.0);
                frames.push((t, shown.at.map(|a| (a, m.fade)), shown.wake));
                t = (t + next(wait)).min(news.get(k).map_or(f64::INFINITY, |n| n.0));
            }
            frames
        };
        let fine = run(&|_| 1.0 / 480.0, 9.0);
        let asked = run(&|wait| wait.max(1.0 / 960.0), 9.0);
        let at = |t: f64| fine.iter().rfind(|f| f.0 <= t + 1e-9).and_then(|f| f.1).map(|(a, _)| a);
        let word = |p: Pos2| words.iter().position(|w| w.1.expand(0.01).contains(p));
        for (i, (t, drawn, _)) in asked.iter().enumerate() {
            let (Some((p, _)), Some(q)) = (drawn, at(*t)) else { continue };
            assert!((*p - q).length() < 0.5 || word(*p) != word(q), "at {t:.3} s drawn at {p:?}, steady frames at {q:?}");
            // Between this frame and the next, on the same word (a hop over the space between words is drawn at once).
            let until = asked.get(i + 1).map_or(9.0, |f| f.0);
            let on = fine.iter().filter(|f| f.0 > *t && f.0 < until).filter_map(|f| f.1).filter(|(a, _)| word(*a) == word(*p));
            let behind = on.map(|(a, _)| (a - *p).length()).fold(0.0, f32::max);
            assert!(behind <= 1.0, "between {t:.3} and {until:.3} s the pointer moved {behind} px undrawn");
        }
        // A hop to the next word drawn within a few milliseconds of the steady frames' drawing it.
        let hops = |frames: &[Frame]| {
            frames.windows(2).filter_map(|w| Some((w[1].0, word(w[0].1?.0)?, word(w[1].1?.0)?))).filter(|h| h.1 != h.2).map(|h| (h.0, h.2)).collect::<Vec<_>>()
        };
        let (steady_hops, asked_hops) = (hops(&fine), hops(&asked));
        assert_eq!(steady_hops.iter().map(|h| h.1).collect::<Vec<_>>(), asked_hops.iter().map(|h| h.1).collect::<Vec<_>>());
        for (a, b) in steady_hops.iter().zip(&asked_hops) {
            assert!((b.0 - a.0).abs() < 0.01, "to word {}: at {:.4} s, steady frames at {:.4} s", a.1, b.0, a.0);
        }
        // Moving at the reader's pace, as often as a 60 Hz screen draws, or more.
        let count = |a: f64, b: f64| asked.iter().filter(|f| (a..b).contains(&f.0)).count();
        assert!(count(1.0, 2.0) >= 60, "{} frames reading", count(1.0, 2.0));
        // The range's edge behind eases over 0.64 s from about a word back: a few frames a second two seconds into the
        // silence, at rest within four.
        let last = news.last().unwrap().0;
        assert!(count(last + 2.0, last + 3.0) <= 10, "{} frames 2 s into the silence", count(last + 2.0, last + 3.0));
        let resting = asked.iter().find(|f| f.0 > last && f.2 == Wake::Rest).map(|f| f.0 - last);
        assert!(resting.is_some_and(|s| s < 4.0), "at rest {resting:?} s after the last report");
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

    #[test]
    fn reports_from_a_silence_the_reader_keeps_are_not_followed() {
        let params = Params::default();
        let at = |loc: Loc, word: u32, t: f32| Heard { t, ..heard(loc, word) };
        let mut m = Marker::default();
        // Reading 2:7:1-2 (words 100-101), then silent from 10.0 s of audio (its last loud level).
        assert!(m.heard(at((2, 7, 1), 100, 9.0), 9.0, &params).is_some());
        assert!(m.heard(at((2, 7, 2), 101, 9.6), 9.6, &params).is_some());
        m.voice(Heard2 { present: false, quiet: true }, Some(10.0), 10.2);
        // The last word before the silence, decided late (10.3 s, its evidence of 10.15 s): followed.
        assert!(m.heard(at((2, 7, 3), 102, 10.3), 10.3, &params).is_some());
        // Words decided 1.5 s and more into the silence: not followed, whatever their state; the same word still is.
        assert!(m.heard(at((2, 7, 4), 103, 11.5), 11.5, &params).is_none());
        assert!(m.heard(Heard { state: "Drifting".into(), ..at((2, 7, 5), 104, 12.0) }, 12.0, &params).is_none());
        assert!(m.heard(Heard { state: "Paused".into(), ..at((2, 7, 3), 102, 12.3) }, 12.3, &params).is_some());
        // The voice back: followed again.
        m.voice(Heard2 { present: true, quiet: false }, Some(12.6), 12.6);
        assert!(m.heard(at((2, 7, 4), 103, 12.8), 12.8, &params).is_some());
        // The preview's voice (no audio time) holds nothing.
        m.voice(Heard2 { present: false, quiet: true }, None, 13.0);
        assert!(m.heard(at((2, 7, 5), 104, 15.0), 15.0, &params).is_some());
    }

    #[test]
    fn the_range_of_a_surahs_first_ayah_never_reaches_back_over_its_bismillah_line() {
        // Al-Mursalat's Bismillah line over its first ayahs' line, right to left; 77:1:1 is word 1000 here.
        let line = |y: f32, locs: &[Loc]| -> Vec<Slot> {
            (0..).zip(locs).map(|(k, &l)| (l, Rect::from_min_size(pos2(200.0 - 60.0 * k as f32, y), vec2(50.0, 40.0)), y + 28.0, 20.0)).collect()
        };
        let mut words = line(0.0, &[(77, 0, 1), (77, 0, 2), (77, 0, 3), (77, 0, 4)]);
        words.extend(line(40.0, &[(77, 1, 1), (77, 1, 2), (77, 2, 1)]));
        let size = (30.0, 5.0, 10.0);
        // Reading 77:1:2, the range reaching back past the ayah's first word: on the ayah's line only.
        let h = heard((77, 1, 2), 1001);
        let place = |c: f32| point(c, &h, &words, 0.2);
        let (at, i) = place(1001.5).unwrap();
        let rects = range_rects(&words, place, (999.5, 1002.0), (at, i), size);
        assert_eq!(rects.len(), 1);
        assert!(rects[0].top() > 40.0, "{rects:?}");
    }

    #[test]
    fn the_pointer_keeps_to_the_line_until_the_reader_is_heard_on_the_next() {
        let words = page();
        let colors = crate::theme::Colors::of(&quran_core::settings::Settings::default(), false);
        let ctx = egui::Context::default();
        // Frames at 60 a second from `from` for `secs`: where the dot was drawn and how faded it was.
        let frames = |m: &mut Marker, settings: &Settings, from: f64, secs: f64| {
            let mut out = Vec::new();
            for k in 1..=(secs * 60.0).round() as i32 {
                let now = from + f64::from(k) / 60.0;
                let mut output = ctx.run_pass(egui::RawInput { time: Some(now), ..Default::default() }, |ctx| {
                    let layer = |n| LayerId::new(egui::Order::Middle, egui::Id::unique(n));
                    let stage = Stage { now, dt: 1.0 / 60.0, area: Rect::from_min_size(pos2(0.0, 0.0), vec2(300.0, 100.0)), layers: (layer("r"), layer("p")), scale: 1.0, scrolls: false };
                    if let Some(at) = m.frame(ctx, &stage, settings, &colors, &words).at {
                        out.push((at.y, m.fade));
                    }
                });
                output.textures_delta.clear();
            }
            out
        };
        // Heard on the first line's last word (102), its last letter, the plan running 0.35 into the next word, the next
        // line's first; or already in it when shown (the display lead).
        for (now, stop) in [(102.8, true), (103.1, true), (102.8, false)] {
            let to_next_line = Plan { seq: 1, age: 0.0, ev: (102, 3), ev_end: 102.95, now, knots: vec![(0.3, 0.15), (0.8, 0.5)], hold: None };
            let mut settings = Settings::default();
            (settings.motion.follow_tajweed, settings.motion.stale_wait_ms, settings.motion.stop_at_line_end) = (true, 0, stop);
            let params = Params::of(&settings);
            let mut m = Marker::default();
            m.heard(Heard { letter: 3, plan: Some(to_next_line.clone()), ..heard((2, 7, 3), 102) }, 0.0, &params);
            let drawn = frames(&mut m, &settings, 0.0, 1.5);
            let next = drawn.iter().any(|&(y, _)| y > 40.0);
            assert_eq!(next, !stop, "stop {stop}, now {now}: {drawn:?}");
            if stop {
                // On its line throughout, faded in once and never over again.
                assert!(drawn[20..].iter().all(|&(y, fade)| (y - 24.0).abs() < 0.5 && fade == 1.0), "{drawn:?}");
                // Heard on the next line: it goes there (fading over, as on any line change).
                let there = Plan { seq: 2, age: 0.0, ev: (103, 0), ev_end: 103.2375, now: 103.1, knots: vec![(0.3, 0.1)], hold: None };
                m.heard(Heard { letter: 0, plan: Some(there), ..heard((2, 7, 4), 103) }, 1.5, &params);
                let after = frames(&mut m, &settings, 1.5, 1.0);
                assert!(after.last().is_some_and(|&(y, fade)| (y - 64.0).abs() < 0.5 && fade == 1.0), "{after:?}");
            }
        }
    }

    #[test]
    fn a_scroll_is_no_line_change_and_moves_the_smudge_with_the_page() {
        let (mut settings, ctx) = (Settings::default(), egui::Context::default());
        settings.pointer.smudge = 6;
        let colors = crate::theme::Colors::of(&quran_core::settings::Settings::default(), false);
        let mut m = Marker::default();
        m.heard(heard((2, 7, 2), 101), 0.0, &Params::of(&settings));
        // Still for half a second, then a wheel notch's worth at once (more than a line), then a smooth scroll.
        let shifts = (0..30).map(|_| 0.0).chain([-45.0]).chain((0..30).map(|_| -1.5));
        let (mut dy, mut drawn) = (0.0f32, Vec::new());
        for (k, shift) in (1..).zip(shifts) {
            dy += shift;
            let words: Vec<Slot> = page().into_iter().map(|(l, r, b, e)| (l, r.translate(vec2(0.0, dy)), b + dy, e)).collect();
            let now = f64::from(k) / 60.0;
            let mut output = ctx.run_pass(egui::RawInput { time: Some(now), ..Default::default() }, |ctx| {
                let layer = |n| LayerId::new(egui::Order::Middle, egui::Id::unique(n));
                let area = Rect::from_min_size(pos2(0.0, -200.0), vec2(300.0, 400.0));
                let stage = Stage { now, dt: 1.0 / 60.0, area, layers: (layer("r"), layer("p")), scale: 1.0, scrolls: false };
                let at = m.frame(ctx, &stage, &settings, &colors, &words).at.unwrap();
                drawn.push((at.y - dy, m.fade, m.dot.path_ends()));
            });
            output.textures_delta.clear();
        }
        // On its line on the page throughout, never faded over, its path on the page where the head is.
        let (y, ..) = drawn[29];
        assert!(drawn[29..].iter().all(|&(at, fade, _)| (at - y).abs() < 0.01 && fade == 1.0), "{drawn:?}");
        assert!(drawn[29..].iter().all(|&(_, _, ends)| ends.is_none_or(|(a, b)| (a.y - b.y).abs() < 0.01)), "{drawn:?}");
    }

    #[test]
    fn with_the_defaults_reading_lines_in_view_mid_page_never_scrolls() {
        let (settings, ctx) = (Settings::default(), egui::Context::default());
        let colors = crate::theme::Colors::of(&quran_core::settings::Settings::default(), false);
        let params = Params::of(&settings);
        // Four lines of three words, 40 apart from a fifth of the way down a 400-high view; words 100.. of 2:7.
        let words: Vec<Slot> = (0..12u16)
            .map(|k| {
                let rect = Rect::from_min_size(pos2(200.0 - 60.0 * f32::from(k % 3), 80.0 + 40.0 * f32::from(k / 3)), vec2(50.0, 40.0));
                ((2, 7, k + 1), rect, rect.top() + 28.0, 20.0)
            })
            .collect();
        let mut m = Marker::default();
        let mut now = 0.0;
        // The first three lines read, two reports a word, 0.32 s apart.
        for k in 0..18u16 {
            let h = Heard { letter: (k % 2) * 2, ..heard((2, 7, k / 2 + 1), 100 + u32::from(k / 2)) };
            m.heard(h, now, &params);
            for _ in 0..19 {
                now += 1.0 / 60.0;
                let mut output = ctx.run_pass(egui::RawInput { time: Some(now), ..Default::default() }, |ctx| {
                    let layer = |n| LayerId::new(egui::Order::Middle, egui::Id::unique(n));
                    let area = Rect::from_min_size(pos2(0.0, 0.0), vec2(300.0, 400.0));
                    let stage = Stage { now, dt: 1.0 / 60.0, area, layers: (layer("r"), layer("p")), scale: 1.0, scrolls: true };
                    let shown = m.frame(ctx, &stage, &settings, &colors, &words);
                    assert_eq!(shown.steer.scroll, 0.0, "at {now:.2} s: {:?}", shown.at);
                });
                output.textures_delta.clear();
            }
        }
    }

    #[test]
    fn with_the_voice_held_frames_when_asked_follow_the_reader_as_steady_frames_do() {
        // The stand-in listener of the headless checks: a report every 0.32 s (two a word), the voice's level every
        // 40 ms through the link's presence, quiet a second before the reports stop and back a second before they come
        // again, and a report of the next word heard in the silence. The run-on held while the voice is quiet:
        // frames drawn when asked show the pointer where steady fine frames do, the silence draws few, and the voice's
        // return is not run on from as if it had been heard since the last frame.
        use super::super::presence::Presence;
        enum News {
            Report(Heard),
            Voice(Heard2, Option<f32>),
        }
        let words: Vec<Slot> = (0..64u16)
            .map(|k| {
                let rect = Rect::from_min_size(pos2(900.0 - 85.0 * f32::from(k % 10), 80.0 + 60.0 * f32::from(k / 10)), vec2(75.0, 50.0));
                ((2, 7, k + 1), rect, rect.top() + 36.0, 25.0)
            })
            .collect();
        let mut settings = Settings::default();
        let motion = &mut settings.motion;
        (motion.hold_when_quiet, motion.quiet_dim, motion.ayah_fade_delay_ms, settings.range.enabled) = (true, 50, 5000, false);
        let params = Params::of(&settings);
        let (mut news, mut presence) = (Vec::new(), Presence::default());
        for f in 1..400u16 {
            let at = f32::from(f) * 0.04;
            let db = if (6.0..10.0).contains(&at) || f % 3 == 0 { -48.0 } else { -22.0 };
            if let Some(h) = presence.push(at, db) {
                news.push((f64::from(at) - 1.0, News::Voice(h, presence.last_loud())));
            }
        }
        let (mut t, mut sent, mut k) = (0.0f64, 0u16, 0u16);
        while k < 30 {
            t += 0.32;
            let report = |letter| News::Report(Heard { t: t as f32, conf: 0.97, speed: 8.2, letter, ..heard((2, 7, k + 1), 100 + u32::from(k)) });
            if (6.0..10.0).contains(&t) {
                if (t - 7.5).abs() < 0.17 {
                    news.push((t, report(0)));
                }
                continue;
            }
            news.push((t, report((sent % 2) * 2)));
            sent += 1;
            k += u16::from(sent % 2 == 0);
        }
        news.sort_by(|a, b| a.0.total_cmp(&b.0));
        let run = |asked: bool| {
            let (ctx, mut m) = (egui::Context::default(), Marker::default());
            let (mut now, mut i, mut frames) = (-0.9f64, 0, Vec::new());
            while now < 15.0 {
                while i < news.len() && news[i].0 <= now + 1e-9 {
                    match &news[i].1 {
                        News::Report(h) => drop(m.heard(h.clone(), now, &params)),
                        News::Voice(h, loud) => m.voice(*h, *loud, now),
                    }
                    i += 1;
                }
                let (shown, wait) = shown_at(&ctx, &mut m, &settings, &words, (now, Some(1.0 / 480.0)), 800.0);
                frames.push((now, shown.at));
                now = (now + if asked { wait.max(1.0 / 960.0) } else { 1.0 / 480.0 }).min(news.get(i).map_or(f64::INFINITY, |n| n.0));
            }
            frames
        };
        let (fine, asked) = (run(false), run(true));
        let word = |p: Pos2| words.iter().position(|w| w.1.expand(0.01).contains(p));
        for (t, p) in &asked {
            let q = fine.iter().rfind(|f| f.0 <= t + 1e-9).and_then(|f| f.1);
            if let (Some(p), Some(q)) = (p, q)
                && word(*p) == word(q)
            {
                assert!((*p - q).length() < 0.75, "at {t:.3} s drawn at {p:?}, steady frames at {q:?}");
            }
        }
        // Reading, as often as a 60 Hz screen draws or more; the silence (from a second after the last report until
        // the report heard in it, and from a second after that until the voice is back) a few frames a second.
        let count = |a: f64, b: f64| asked.iter().filter(|f| (a..b).contains(&f.0)).count();
        assert!(count(2.0, 5.0) >= 180, "{} frames reading", count(2.0, 5.0));
        assert!(count(6.8, 7.3) + count(8.4, 9.0) <= 30, "{} frames in the silence", count(6.8, 7.3) + count(8.4, 9.0));
    }
}
