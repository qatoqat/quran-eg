//! The preview's player: a scene of `script` played into a marker on a clock of its own, as the live pointer plays
//! what the listener sends, over and over, noting what the real code decided for the subtitle.

use std::collections::VecDeque;

use egui::{LayerId, Pos2, Rect};
use quran_core::settings::Voice;

use super::follow::{ADVANCE, AWAY, Stray};
use super::marker::{Marker, Shown, Stage, position};
use super::motion::{Jump, Params, RUN_ON};
use super::presence::Heard;
use super::script::{Cadence, ELSEWHERE, Moment, Scene, Script, TICK, Update};
use crate::reader::Slot;
use crate::theme::Colors;

/// Seconds a tick of the listener stays, fading.
const TICK_FADE: f64 = 2.4;
/// Seconds a slip stays in the subtitle, and a decision after its moment while the reader reads on.
const SLIP_SHOWN: f64 = 2.5;
const LINGER: f64 = 1.5;
/// Seconds the page-turn card shows before the far scene plays again; without keeping the line in view (nothing
/// to turn), how long it goes on after the reader went elsewhere.
const TURN: f64 = 1.6;
const STAYS: f64 = 4.0;

/// Where the player draws: the canvas (as far as it shows) and its layer, what a point of the page is worth there,
/// the passage's words.
pub struct Canvas<'a> {
    pub area: Rect,
    pub layer: LayerId,
    pub scale: f32,
    pub slots: &'a [Slot],
}

/// A scene of the script playing into a marker on a clock of its own.
pub struct Player {
    marker: Marker,
    cadence: Cadence,
    script: Script,
    updates: Vec<Update>,
    /// What the voice's level says (with settings that ask for it), and the next of it to hear.
    voices: Vec<(f64, Heard)>,
    next_voice: usize,
    /// Seconds played (never reset), and the clock time the scene's start is at.
    clock: f64,
    origin: f64,
    /// The next message to hear.
    next: usize,
    /// Each word position heard on the passage lately, and when (clock time).
    marks: VecDeque<(f64, f32)>,
    /// What the real code decided: the jump at the first message of a moment's stretch, the stretch whose
    /// mishearing it followed, the slip it reported (scene time, and how long it lasted) and when it turned the
    /// page.
    jump: Option<(usize, Option<Jump>)>,
    followed: Option<usize>,
    slip: Option<(f64, Stray, f64)>,
    turned: Option<f64>,
    /// Where the dot was last frame, and whether it was faded at an ayah's end.
    at: Option<Pos2>,
    faded: bool,
    /// The passage's words' boxes last frame: when they move, it was laid out again.
    laid: Vec<Rect>,
}

/// Seconds after the reader goes on elsewhere until the far place is followed: after a second without a wait,
/// else after the wait (a second at least) once its run has advanced far enough, heard tick by tick.
fn follows(voice: &Voice) -> f64 {
    let advanced = ((f64::from(ADVANCE) - f64::from(ELSEWHERE.from)) / f64::from(ELSEWHERE.pace) / TICK).ceil() * TICK;
    match voice.far_wait_ms {
        0 => AWAY,
        ms => (f64::from(ms) / 1000.0).max(AWAY).max(advanced),
    }
}

/// What the pointer does over a held letter: it runs on at the reader's pace (`pace`) as far as `max_ahead` lets
/// it, for a second at most.
fn run_on(max_ahead: u8, pace: f32) -> String {
    let (most, can) = (f32::from(max_ahead) / 10.0, pace * RUN_ON);
    if max_ahead == 0 {
        "stays on it (no running ahead)".into()
    } else if most > can + 0.05 {
        format!("runs on {can:.1} word ({RUN_ON} s at most)")
    } else {
        format!("runs on {most:.1} word at most")
    }
}

impl Player {
    /// Reading (which no setting lengthens), heard at `cadence`.
    pub fn new(cadence: Cadence) -> Self {
        let script = Script::of(Scene::Read, &Voice::default());
        Player {
            marker: Marker::default(),
            cadence,
            updates: script.updates(cadence),
            voices: script.voices(),
            next_voice: 0,
            script,
            clock: 0.0,
            origin: 0.0,
            next: 0,
            marks: VecDeque::new(),
            jump: None,
            followed: None,
            slip: None,
            turned: None,
            at: None,
            faded: false,
            laid: Vec::new(),
        }
    }

    /// Time into the scene.
    pub fn time(&self) -> f64 {
        self.clock - self.origin
    }

    pub fn scene(&self) -> Scene {
        self.script.scene
    }

    /// Plays `scene` (as long as `voice` has it) from its start: a new marker (the dot fades in with nothing behind
    /// it to smear).
    pub fn restart(&mut self, scene: Scene, voice: &Voice) {
        self.script = Script::of(scene, voice);
        self.updates = self.script.updates(self.cadence);
        self.voices = self.script.voices();
        self.marker = Marker::default();
        (self.origin, self.next, self.next_voice) = (self.clock, 0, 0);
        self.marks.clear();
        (self.jump, self.followed, self.slip, self.turned, self.at, self.faded) = (None, None, None, None, None, false);
        self.laid.clear();
    }

    /// Hears the other listener from now on.
    pub fn hear(&mut self, cadence: Cadence) {
        self.cadence = cadence;
        self.updates = self.script.updates(cadence);
        self.next = self.updates.partition_point(|u| u.at <= self.time());
    }

    /// Plays `dt` seconds more (0 while paused): the marker hears what the listener sent meanwhile.
    pub fn advance(&mut self, dt: f32, voice: &Voice) {
        self.clock += f64::from(dt);
        let (t, params) = (self.time(), Params::of(voice));
        while let Some(&(at, heard)) = self.voices.get(self.next_voice).filter(|v| v.0 <= t) {
            self.marker.voice(heard, None, self.origin + at);
            self.next_voice += 1;
        }
        while let Some(u) = self.updates.get(self.next).filter(|u| u.at <= t) {
            let at = self.origin + u.at;
            let jump = self.marker.heard(u.heard.clone(), at, &params);
            let moment = self.script.stretches()[u.stretch].moment;
            if !matches!(moment, Moment::Plain | Moment::On) && self.jump.is_none_or(|(i, _)| i != u.stretch) {
                self.jump = Some((u.stretch, jump));
            }
            if u.heard.loc.0 == 1 {
                self.marks.push_back((at, position(&u.heard)));
            }
            self.next += 1;
        }
        while self.marks.front().is_some_and(|(at, _)| self.clock - at > TICK_FADE) {
            self.marks.pop_front();
        }
    }

    /// Draws the pointer on the canvas (after the words) and notes what the real code decided.
    pub fn frame(&mut self, ctx: &egui::Context, dt: f32, canvas: &Canvas, voice: &Voice, colors: &Colors) -> Shown {
        if !self.laid.iter().copied().eq(canvas.slots.iter().map(|s| s.1)) {
            if !self.laid.is_empty() {
                self.marker.relaid();
            }
            self.laid = canvas.slots.iter().map(|s| s.1).collect();
        }
        let stage = Stage { now: self.clock, dt, area: canvas.area, layers: (canvas.layer, canvas.layer), scale: canvas.scale, scrolls: false };
        let shown = self.marker.frame(ctx, &stage, voice, colors, canvas.slots);
        let t = self.time();
        let (i, s) = self.script.at(t);
        if shown.steer.go_to.is_some() {
            if s.moment == Moment::Away {
                self.turned = self.turned.or(Some(t));
            } else {
                self.followed = Some(i);
            }
        }
        if let Some(slip) = shown.slip {
            self.slip = Some((t, slip, self.clock - slip.since));
        }
        (self.at, self.faded) = (shown.at, shown.faded);
        shown
    }

    /// The reader chose in the toast: to go to the far place now (the page turns), or to stay.
    pub fn choose(&mut self, go: bool) {
        if self.marker.choose(go).is_some() {
            self.turned = self.turned.or(Some(self.time()));
        }
    }

    /// Where the reader really is (words into the passage), if on it.
    pub fn truth(&self) -> Option<f32> {
        self.script.truth(self.time())
    }

    /// The listener's word positions lately (as the marker places them), each with how far it has faded (0..1).
    pub fn ticks(&self) -> impl Iterator<Item = (f32, f32)> + '_ {
        self.marks.iter().map(|&(at, c)| (c, ((self.clock - at) / TICK_FADE) as f32))
    }

    /// Nothing moves on the canvas but what the marker moves: no tick fading, the reader not reading on along it.
    pub fn still(&self) -> bool {
        self.marks.is_empty() && !self.script.moving(self.time())
    }

    /// Seconds until the listener's next message.
    pub fn next_in(&self) -> f64 {
        self.updates.get(self.next).map_or(f64::INFINITY, |u| u.at - self.time())
    }

    pub fn cadence(&self) -> Cadence {
        self.cadence
    }

    /// When the page turned to where the reader went on (scene time).
    pub fn turned(&self) -> Option<f64> {
        self.turned
    }

    /// Where the dot was last frame.
    pub fn dot(&self) -> Option<Pos2> {
        self.at
    }

    /// How long the scene plays: the far scene until the page has turned and its card shown, as `voice` has it.
    pub fn length(&self, voice: &Voice) -> f64 {
        match self.script.moved() {
            Some(moved) if voice.auto_scroll => moved + follows(voice) + TURN,
            Some(moved) => moved + STAYS,
            None => self.script.len(),
        }
    }

    /// When the scene is over, to play again: its length, or once the page turned its card's time after (the far
    /// scene; unturned, a while longer than it could take).
    pub fn end(&self, voice: &Voice) -> f64 {
        match (self.turned, self.script.moved()) {
            (Some(at), _) => at + TURN,
            (None, Some(_)) if voice.auto_scroll => self.length(voice) + STAYS,
            _ => self.length(voice),
        }
    }

    /// What the reader does and what the pointer decided, in a line: only what the real code returned, and the
    /// settings it went by.
    pub fn caption(&self, voice: &Voice) -> String {
        let t = self.time();
        if let Some((_, slip, lasted)) = self.slip.filter(|(at, ..)| t - at < SLIP_SHOWN) {
            return format!("back from {}:{} after {lasted:.1} s: counted as a slip", slip.to.0, slip.to.1);
        }
        // Reading on after a moment played: still what was decided there, for a while.
        let all = self.script.stretches();
        let (i, s) = match self.script.at(t) {
            (i, s) if s.moment == Moment::On => (0..i)
                .rev()
                .find(|j| all[*j].moment != Moment::On)
                .filter(|j| t < all[*j].until() + LINGER)
                .map_or((i, s), |j| (j, &all[j])),
            playing => playing,
        };
        let m = &voice.motion;
        let secs = |ms: u16| format!("{:.1} s", f32::from(ms) / 1000.0);
        let back = format!("fades over (beyond {:.1} back)", f32::from(m.fade_back) / 10.0);
        let decided = match (s.moment, self.jump.filter(|(at, _)| *at == i).and_then(|(_, jump)| jump)) {
            (Moment::Unsure, _) => Some("heard unsure at first: a wide range".to_string()),
            (Moment::AyahEnd, _) if !m.fade_at_ayah_end => Some("stays (fading is off)".into()),
            (Moment::AyahEnd, _) => Some(format!("{} after {}", if self.faded { "faded" } else { "fades" }, secs(m.ayah_fade_delay_ms))),
            (Moment::Held, _) if m.follow_tajweed && m.strict_release => Some("creeps across it, then waits for the next letter".into()),
            (Moment::Held, _) if m.follow_tajweed => Some("creeps across it in its time, then waits".into()),
            (Moment::Held, _) => Some(run_on(m.max_ahead, self.marker.pace())),
            (Moment::Quiet, _) if m.quiet_dim < 100 => Some(format!("holds, and dims to {}% after {}", m.quiet_dim, secs(m.ayah_fade_delay_ms))),
            (Moment::Quiet, _) => Some("holds where it is".into()),
            (Moment::Repeat | Moment::Back, Some(Jump::Respawn)) => Some(back),
            (Moment::Repeat, Some(Jump::Glide)) if m.wait_ms == 0 => Some("eases back at once (no wait)".into()),
            (Moment::Repeat, Some(Jump::Glide)) => Some(format!("waits {}", secs(m.wait_ms))),
            (Moment::Back, Some(Jump::Glide)) => Some(format!("waits {}, range reaching back", secs(m.wait_ms))),
            (Moment::Skip, None) if voice.tuned => Some("held until the reading goes on there (tuned)".into()),
            (Moment::Skip, Some(Jump::Respawn)) => Some(format!("fades over (beyond {})", m.far_jump)),
            (Moment::Skip, Some(Jump::Glide)) => Some(format!("glides (fades over beyond {})", m.far_jump)),
            (Moment::Misheard, _) if self.followed == Some(i) => Some("followed at once (no wait)".into()),
            (Moment::Misheard, _) if self.marker.stray().is_some() => Some("lost, the page stays".into()),
            (Moment::Away, _) if !voice.auto_scroll => Some("stays (keeping in view is off)".into()),
            (Moment::Away, _) => self.marker.stray().map(|stray| match stray.since + f64::from(voice.far_wait_ms) / 1000.0 - self.clock {
                left if left > 0.05 => format!("following in {left:.1} s"),
                _ => format!("following once {ADVANCE} words on"),
            }),
            _ => None,
        };
        match decided {
            Some(decided) => format!("{}: {decided}", s.doing),
            None => s.doing.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use egui::epaint::Shape;
    use egui::{pos2, vec2};
    use quran_core::settings::Settings;

    use super::super::dock;
    use super::super::script::FIRST;
    use super::*;

    const FRAME: f64 = 1.0 / 60.0;

    fn colors() -> Colors {
        Colors::of(&Settings::default(), false)
    }

    /// The test canvas, and the passage on it at its measured widths (no fonts) for a page at 50 points a glyph em.
    fn canvas() -> (Rect, dock::Placed) {
        let area = Rect::from_min_size(pos2(14.0, 600.0), vec2(324.0, 156.0));
        let words = Rect::from_min_max(area.min + vec2(12.0, 6.0), area.max - vec2(12.0, 22.0));
        (area, dock::place(words, 50.0, 2, None))
    }

    /// Words into the passage at `at` (on a word's line).
    fn words_at(placed: &dock::Placed, at: Pos2) -> f32 {
        let i = placed.slots.iter().position(|(_, r, ..)| r.y_range().contains(at.y) && at.x >= r.left() - 1.0 && at.x <= r.right() + 1.0).expect("on a word");
        let r = placed.slots[i].1;
        i as f32 + (r.right() - at.x) / r.width()
    }

    /// A frame of the player: scene time, the dot (in words into the passage), faded, a slip reported, the page
    /// turned or a mishearing followed, the range's rectangles drawn and the most opaque of their fills.
    #[derive(Debug)]
    struct Frame {
        t: f64,
        x: Option<f32>,
        faded: bool,
        slip: bool,
        go_to: bool,
        rects: usize,
        alpha: u8,
    }

    /// The jump decided at each stretch's first message.
    type Jumps = Vec<(usize, Option<Jump>)>;

    /// Plays `scene` to its time `until` at 60 frames a second; the frames, the jumps, and the subtitle at the end.
    fn play(voice: &Voice, cadence: Cadence, scene: Scene, until: f64) -> (Vec<Frame>, Jumps, String) {
        let ctx = egui::Context::default();
        let (area, placed) = canvas();
        let (mut player, mut frames, mut jumps) = (Player::new(cadence), Vec::new(), Vec::new());
        player.restart(scene, voice);
        let mut now = 100.0;
        while player.time() < until {
            let mut shown = None;
            let input = egui::RawInput { time: Some(now), ..Default::default() };
            let mut output = ctx.run_pass(input, |ctx| {
                player.advance(FRAME as f32, voice);
                let on = Canvas { area, layer: LayerId::background(), scale: 0.5, slots: &placed.slots };
                shown = Some(player.frame(ctx, FRAME as f32, &on, voice, &colors()));
            });
            output.textures_delta.clear();
            let shown = shown.unwrap();
            let fills: Vec<u8> = output.shapes.iter().filter_map(|s| if let Shape::Rect(r) = &s.shape { Some(r.fill.a()) } else { None }).filter(|&a| a > 0).collect();
            let x = shown.at.map(|at| words_at(&placed, at));
            let (rects, alpha) = (fills.len(), fills.iter().copied().max().unwrap_or(0));
            frames.push(Frame { t: player.time(), x, faded: shown.faded, slip: shown.slip.is_some(), go_to: shown.steer.go_to.is_some(), rects, alpha });
            if let Some(jump) = player.jump.filter(|j| jumps.last() != Some(j)) {
                jumps.push(jump);
            }
            now += FRAME;
        }
        let caption = player.caption(voice);
        (frames, jumps, caption)
    }

    /// The jump decided at the first message of the stretch of `scene` there for `moment`.
    fn jump(jumps: &[(usize, Option<Jump>)], scene: Scene, moment: Moment) -> Option<Jump> {
        let i = Script::of(scene, &manual()).stretches().iter().position(|s| s.moment == moment).unwrap();
        jumps.iter().find(|j| j.0 == i).and_then(|j| j.1)
    }

    /// The start of the stretch of `scene` (as `voice` has it) there for `moment`.
    fn start(voice: &Voice, scene: Scene, moment: Moment) -> f64 {
        let script = Script::of(scene, voice);
        script.stretches().iter().find(|s| s.moment == moment).unwrap().start()
    }

    /// The dot's positions between times `from` and `to`.
    fn xs(frames: &[Frame], from: f64, to: f64) -> Vec<f32> {
        frames.iter().filter(|f| (from..to).contains(&f.t)).filter_map(|f| f.x).collect()
    }

    /// The settings' defaults with the motion as set (tuned movement off), which these scenes' captions describe.
    fn manual() -> Voice {
        Voice { tuned: false, ..Voice::default() }
    }

    fn with(change: impl Fn(&mut Voice)) -> Voice {
        let mut voice = manual();
        change(&mut voice);
        voice
    }

    const BOTH: [Cadence; 2] = [Cadence::Even, Cadence::Heard];

    #[test]
    fn the_real_pointer_appears_and_fades_at_the_ayah_end_after_any_delay() {
        for cadence in BOTH {
            let (frames, jumps, _) = play(&manual(), cadence, Scene::Read, 4.2);
            assert_eq!(jumps[0], (1, Some(Jump::Respawn)), "{cadence:?}: appears");
            // Read three lines on: the dot goes along all of them.
            let (frames_read, ..) = play(&manual(), cadence, Scene::Read, 12.4);
            assert!(frames_read.iter().filter_map(|f| f.x).fold(0.0, f32::max) > 11.0, "{cadence:?}");
            assert!(frames.iter().all(|f| !f.faded), "{cadence:?}: a breath is not a pause");
            for delay in [0u16, 1000, 4000, 10_000] {
                let voice = with(|v| v.motion.ayah_fade_delay_ms = delay);
                let pause = start(&voice, Scene::AyahEnd, Moment::AyahEnd);
                let (frames, _, _) = play(&voice, cadence, Scene::AyahEnd, Script::of(Scene::AyahEnd, &voice).len());
                let faded: Vec<f64> = frames.iter().filter(|f| f.faded).map(|f| f.t).collect();
                let (first, last) = (faded.first().copied().unwrap_or(-1.0), faded.last().copied().unwrap_or(-1.0));
                let delay = f64::from(delay) / 1000.0;
                // Noticed at once by the perfect listener, two ticks late by the real one; faded until the reader goes on.
                assert!(first >= pause + delay - 0.05 && first <= pause + delay + 0.8 && last - first >= 0.9, "{cadence:?} {delay} s: {first}..{last} from {pause}");
            }
            let (frames, ..) = play(&with(|v| v.motion.fade_at_ayah_end = false), cadence, Scene::AyahEnd, 6.0);
            assert!(!frames.iter().any(|f| f.faded), "{cadence:?}: never faded");
        }
    }

    #[test]
    fn a_held_letter_is_run_on_from_as_far_as_allowed_for_a_second() {
        for cadence in BOTH {
            let script = Script::of(Scene::Hold, &manual());
            let i = script.stretches().iter().position(|s| s.moment == Moment::Held).unwrap();
            let (held, letter) = (script.stretches()[i].start(), script.updates(cadence).into_iter().find(|u| u.stretch == i).unwrap());
            let heard = position(&letter.heard) - FIRST as f32;
            let ran = |tenths: u8| {
                let (frames, ..) = play(&with(|v| v.motion.max_ahead = tenths), cadence, Scene::Hold, held + 2.4);
                xs(&frames, held + 2.3, held + 2.35)[0] - heard
            };
            let (none, half, one, most) = (ran(0), ran(5), ran(10), ran(30));
            assert!(none < 0.05 && (half - 0.5).abs() < 0.1 && (one - 1.0).abs() < 0.12, "{cadence:?}: {none} {half} {one}");
            // Past what a second at the reader's pace takes it: as far as that, and the subtitle says so.
            assert!(most > one - 0.05 && most < 1.25, "{cadence:?}: {most}");
            let (.., caption) = play(&with(|v| v.motion.max_ahead = 30), cadence, Scene::Hold, held + 1.0);
            assert!(caption.ends_with("word (1 s at most)"), "{caption}");
        }
    }

    #[test]
    fn with_tajweed_timing_a_held_letter_is_crept_across_and_a_breath_holds_and_dims() {
        let voice = with(|v| (v.motion.follow_tajweed, v.motion.quiet_dim) = (true, 35));
        for cadence in BOTH {
            let script = Script::of(Scene::Hold, &voice);
            let (held, quiet) = (start(&voice, Scene::Hold, Moment::Held), start(&voice, Scene::Hold, Moment::Quiet));
            let (frames, _, caption) = play(&voice, cadence, Scene::Hold, quiet + 2.2);
            // The held letter (4.75 words in): crept across, never into the next word, while it is held.
            let holding = xs(&frames, held + 0.4, held + 2.5);
            assert!(holding.iter().all(|x| *x < 5.0) && holding.last() > holding.first(), "{cadence:?}: {holding:?}");
            // The breath inside the ayah: the dot holds where it is and the range dims after the delay.
            let breath = xs(&frames, quiet + 0.6, quiet + 2.2);
            assert!(breath.iter().all(|x| (x - breath[0]).abs() < 0.02), "{cadence:?}: {breath:?}");
            let alpha = |from: f64, to: f64| frames.iter().filter(|f| (from..to).contains(&f.t)).map(|f| f.alpha).max().unwrap();
            assert!(f32::from(alpha(quiet + 1.9, quiet + 2.2)) < 0.6 * f32::from(alpha(quiet - 0.6, quiet)), "{cadence:?}: dims");
            assert!(caption.ends_with("dims to 35% after 1.0 s"), "{caption}");
            assert!(script.stretches().len() > Script::of(Scene::Hold, &manual()).stretches().len(), "the breath only with the level");
        }
    }

    #[test]
    fn a_repeat_waits_as_long_as_set_then_eases_back() {
        for cadence in BOTH {
            for wait in [0u16, 1500, 5000] {
                let voice = with(|v| v.motion.wait_ms = wait);
                let script = Script::of(Scene::Slips, &voice);
                let i = script.stretches().iter().position(|s| s.moment == Moment::Repeat).unwrap();
                let (repeat, letter) = (script.stretches()[i].start(), script.updates(cadence).into_iter().find(|u| u.stretch == i).unwrap());
                let heard = position(&letter.heard) - FIRST as f32;
                let (frames, jumps, _) = play(&voice, cadence, Scene::Slips, repeat + f64::from(wait) / 1000.0 + 1.2);
                assert_eq!(jump(&jumps, Scene::Slips, Moment::Repeat), Some(Jump::Glide), "{cadence:?}");
                let wait = f64::from(wait) / 1000.0;
                let before = xs(&frames, repeat - 0.1, repeat + 0.02).last().copied().unwrap();
                // Not back while it waits; once the wait is over, back where the reader is heard (and run on from).
                let waiting = xs(&frames, repeat, repeat + wait - 0.05);
                assert!(waiting.iter().all(|x| *x >= before - 1e-3), "{cadence:?} {wait}: {waiting:?}");
                let after = xs(&frames, repeat + wait + 1.0, repeat + wait + 1.2);
                let back = heard + 0.5;
                assert!(after.iter().all(|x| (x - back).abs() < 0.1 && *x < before - 0.15), "{cadence:?} {wait}: from {before} to {after:?}, not {back}");
            }
        }
    }

    #[test]
    fn the_slips_are_decided_by_their_settings() {
        for cadence in BOTH {
            let len = Script::of(Scene::Slips, &manual()).len();
            let slips = |voice: Voice| play(&voice, cadence, Scene::Slips, len).1;
            assert_eq!(jump(&slips(with(|v| v.motion.fade_back = 5)), Scene::Slips, Moment::Repeat), Some(Jump::Respawn), "{cadence:?}");
            let jumps = slips(manual());
            assert_eq!((jump(&jumps, Scene::Slips, Moment::Skip), jump(&jumps, Scene::Slips, Moment::Back)), (Some(Jump::Glide), Some(Jump::Respawn)), "{cadence:?}");
            assert_eq!(jump(&slips(with(|v| v.motion.far_jump = 3)), Scene::Slips, Moment::Skip), Some(Jump::Respawn), "{cadence:?}");
            // Allowed a long step back, the pointer waits on the line after and the range reaches back onto it.
            let voice = with(|v| v.motion.fade_back = 50);
            let (frames, jumps, _) = play(&voice, cadence, Scene::Slips, len);
            assert_eq!(jump(&jumps, Scene::Slips, Moment::Back), Some(Jump::Glide), "{cadence:?}");
            let back = start(&voice, Scene::Slips, Moment::Back);
            assert!(frames.iter().any(|f| f.t > back + 0.05 && f.rects >= 2), "{cadence:?}: two rects");
        }
    }

    #[test]
    fn far_places_are_lost_counted_as_slips_and_followed_after_the_wait() {
        for cadence in BOTH {
            let len = Script::of(Scene::Misheard, &manual()).len();
            let (frames, ..) = play(&manual(), cadence, Scene::Misheard, len);
            assert!(frames.iter().any(|f| f.slip) && !frames.iter().any(|f| f.go_to), "{cadence:?}: a slip, not followed");
            let (frames, ..) = play(&with(|v| v.far_wait_ms = 0), cadence, Scene::Misheard, len);
            assert!(frames.iter().any(|f| f.go_to), "{cadence:?}: no wait, followed");
            // Reading on far away: taken there after the wait, once three words on, when the scene says it would be.
            let moved = Script::of(Scene::Far, &manual()).moved().unwrap();
            for wait in [0.0, 1.0, 5.0, 20.0] {
                let voice = with(|v| v.far_wait_ms = (wait * 1000.0) as u16);
                let expected = moved + follows(&voice);
                let (frames, ..) = play(&voice, cadence, Scene::Far, expected + 1.0);
                let went = frames.iter().find(|f| f.go_to).map(|f| f.t);
                assert!(went.is_some_and(|t| (t - expected).abs() < 0.15), "{cadence:?} {wait} s: {went:?} for {expected}");
            }
            let (frames, ..) = play(&with(|v| v.auto_scroll = false), cadence, Scene::Far, moved + 8.0);
            assert!(!frames.iter().any(|f| f.go_to), "{cadence:?}: never without keeping the line in view");
        }
    }

    #[test]
    fn the_far_scene_plays_until_its_page_has_turned() {
        let voice = manual();
        let mut player = Player::new(Cadence::Heard);
        player.restart(Scene::Far, &voice);
        assert_eq!(player.end(&voice), player.length(&voice) + STAYS, "not turned yet");
        player.turned = Some(9.0);
        assert_eq!(player.end(&voice), 9.0 + TURN);
        let off = with(|v| v.auto_scroll = false);
        player.turned = None;
        assert_eq!(player.end(&off), player.script.moved().unwrap() + STAYS);
        player.restart(Scene::Hold, &voice);
        assert_eq!(player.end(&voice), Script::of(Scene::Hold, &voice).len());
    }
}
