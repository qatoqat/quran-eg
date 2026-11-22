//! How the voice pointer and its accuracy range move between the listener's updates: the follower of
//! quran-voice's `qv-motion` (the quran.com extension moves its pointer the same way). Positions are in
//! words (word index + fraction through the word). Updates arrive every ~320 ms and are bursty (a word
//! held, then several at once), so the pointer runs on at the reader's measured pace, never more than
//! `max_ahead` past the evidence, and eases toward each new estimate along its curve; the range's two
//! edges ease to either side of it, each by its own curve. When the evidence falls behind the pointer,
//! the pointer waits (`wait`) before easing back, while the range's start reaches back to the reader;
//! going back more than `fade_back` words, or forward more than `far_jump`, is a jump: the pointer
//! fades over instead.
//!
//! With "Follow tajweed timing" and a listener that plans (quran-voice's DESIGN §35), the pointer plays the listener's
//! plan instead: on the voice through fluent reading, creeping across a held letter (a madd, a ghunnah), waiting at a
//! word's end once the evidence is stale, resting where the reader stopped, never past the evidenced letter by more
//! than `max_ahead`, the ayah's end or the end of the line the evidence is on (`set_stop`). The plan's clock and the
//! run-on count only while the voice is present (`voice`), and in silence the pointer dims (`dim`). Without plans and
//! voice news it moves exactly as before.
use std::collections::VecDeque;

use quran_core::settings::{Voice, VoiceGlide};

use super::glide::{Ease, Glider};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reading {
    Locked,
    Tentative,
    Drifting,
    Paused,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Params {
    pub pointer: Ease,
    /// The range's edge behind the pointer (toward what was read).
    pub start: Ease,
    /// The range's edge ahead of the pointer.
    pub end: Ease,
    /// Words of range on each side per word of uncertainty.
    pub spread: f32,
    pub fade_back: f32,
    pub far_jump: f32,
    pub max_ahead: f32,
    /// Seconds to wait when the evidence is behind the pointer before easing back.
    pub wait: f32,
    /// A reader pausing at an ayah's end parks the pointer there, and once they have paused this many
    /// seconds it fades out, to fade in again wherever they go on (the next ayah, a step back, another
    /// ayah) instead of gliding over the ayah's end mark. `None`: it never parks.
    pub ayah_fade: Option<f32>,
    /// Follow the listener's plans when they come ("Follow tajweed timing").
    pub plan: bool,
    /// Evidence older than this (s: the plan's age and the time since its message) waits at the evidenced word's end
    /// rather than crossing it on prediction (0: never).
    pub fresh: f32,
    /// After this long without the voice (s), a plan's remaining flow is dropped: only a new seq starts it again.
    pub expire: f32,
    /// Share of the plan's own forward motion the pointer takes at once; its glide does the rest.
    pub ff: f32,
    /// Seconds of the plan its goal is the mean of.
    pub smooth: f32,
    /// The plan's clock and the run-on count only while the voice is present, and with plans nothing moves on after a
    /// long silence until the voice is back ("Hold while you are quiet", always with tajweed timing).
    pub voice: bool,
    /// Tuned movement: the wait before easing back and the glides follow the reader's pace (`Follower::paced`); set by
    /// hand, they are as set.
    pub paced: bool,
    /// Opacity it dims to once the voice and news of the reader going on have been gone `quiet_delay` s (1: never), with
    /// or without the hold; the delay is the fade's at an ayah's end: one delay for fading at stops.
    pub quiet_dim: f32,
    pub quiet_delay: f32,
    /// Words past the evidenced word's end a paused pointer may rest at; further on it eases back to that end.
    pub rest: f32,
    /// When the voice drops with the pointer in a later word than the evidence, ease back to the evidenced word's end.
    pub uncross: bool,
    /// A held letter is left only on news (a new seq), never at its expected end.
    pub strict: bool,
    /// Stay on the line the evidence is on until the reader is heard on the next (the marker sets the stop).
    pub line_stop: bool,
    /// Tuned movement: a jump ahead is held until the reading goes on there (`hold`).
    pub tuned: bool,
    /// Scroll continuously, the line read kept a third of the way down (`keep`; experimental).
    pub continuous: bool,
}

/// Tuned movement's glides and run-on, from the user's recitations as the listener heard them: a report every 0.32 s, a
/// word every 0.64 s (half of them 0.32-0.96 s), 1.3 words a second, the listener ~0.09 s behind the voice. The pointer
/// eases over 1.5 reports (a missed report does not stall it; 0.5 s was the smoothest by eye too), the range's edge
/// behind over a word's time, so it lets go softly; it runs on half a word (the 0.4 s from voice to pointer at that
/// pace) and waits a slow word's time (0.96 s) before easing back.
const TUNED: (u16, u16, u8, u16) = (480, 640, 5, 1000);

impl Params {
    pub fn of(v: &Voice) -> Params {
        let m = &v.motion;
        let ease = |g: VoiceGlide| Ease { curve: g.curve, secs: f32::from(g.ms) / 1000.0 };
        let mut p = Params {
            pointer: ease(m.pointer),
            start: ease(m.range_start),
            end: ease(m.range_end),
            spread: f32::from(v.range.spread) / 10.0,
            fade_back: f32::from(m.fade_back) / 10.0,
            far_jump: f32::from(m.far_jump),
            max_ahead: f32::from(m.max_ahead) / 10.0,
            wait: f32::from(m.wait_ms) / 1000.0,
            ayah_fade: m.fade_at_ayah_end.then(|| f32::from(m.ayah_fade_delay_ms) / 1000.0),
            plan: m.follow_tajweed,
            fresh: f32::from(m.stale_wait_ms) / 1000.0,
            expire: 0.4,
            ff: 0.85,
            smooth: 0.24,
            // Tajweed timing plays the plan as it was benched: on the voice.
            voice: m.hold_when_quiet || m.follow_tajweed,
            paced: false,
            quiet_dim: f32::from(m.quiet_dim.min(100)) / 100.0,
            quiet_delay: f32::from(m.ayah_fade_delay_ms) / 1000.0,
            rest: 0.25,
            uncross: m.retract,
            strict: m.strict_release,
            line_stop: m.stop_at_line_end,
            tuned: v.tuned,
            continuous: m.continuous_scroll,
        };
        if v.tuned {
            let (glide, behind, ahead, wait) = TUNED;
            let smooth = |ms: u16| ease(VoiceGlide { curve: quran_core::settings::VoiceCurve::Smooth, ms });
            (p.pointer, p.start, p.end) = (smooth(glide), smooth(behind), smooth(glide));
            (p.max_ahead, p.wait) = (f32::from(ahead) / 10.0, f32::from(wait) / 1000.0);
            p.paced = true;
        }
        p
    }
}

/// The motion settings' defaults as set, without tuned movement: what the follower's own tests take.
impl Default for Params {
    fn default() -> Self {
        Params::of(&Voice { tuned: false, ..Voice::default() })
    }
}

/// What an update asks the view to do with the drawn pointer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Jump {
    Glide,
    /// Fade the pointer out where it is and in at the new place.
    Respawn,
}

/// Where the pointer and the range's edges are drawn, in words; `start <= x <= end`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Drawn {
    pub x: f32,
    pub start: f32,
    pub end: f32,
}

/// The listener's plan, as a message carries it: where the reader is when the message is shown and how the next half
/// second goes. Positions are in words: word `w`'s letter `l` of `n` spans `w + 0.95 l / n` to `w + 0.95 (l + 1) / n`.
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    /// Grows when the listener's place gets further: the same seq again is no news.
    pub seq: u32,
    /// Seconds from the place's last forward progress to the moment `now` describes.
    pub age: f32,
    /// The place's word (index in the whole Quran) and letter, no display lead, and where that letter ends.
    pub ev: (u32, u16),
    pub ev_end: f32,
    /// Where the reader is when the message is shown.
    pub now: f32,
    /// The path's corners after `now`, as (seconds, words) from it; where they end the reader is waited for.
    pub knots: Vec<(f32, f32)>,
    /// A held letter in the path: when it starts and when its expected time is up, seconds from `now`.
    pub hold: Option<(f32, f32)>,
}

impl Plan {
    /// The path `secs` after the message is shown: from `now` along the knots, waiting where they end.
    pub fn position(&self, secs: f32) -> f32 {
        let secs = secs.max(0.0);
        let mut prev = (0.0, 0.0);
        for &(t, dx) in &self.knots {
            if secs <= t {
                return self.now + if t > prev.0 { prev.1 + (dx - prev.1) * (secs - prev.0) / (t - prev.0) } else { dx };
            }
            prev = (t, dx);
        }
        self.now + prev.1
    }
}

/// The listener's units of speed in a word.
pub const UNITS_PER_WORD: f32 = 6.15;
/// Seconds the pointer runs on at the reader's pace with no news (then it waits for the next).
pub const RUN_ON: f32 = 1.0;
/// Seconds the voice's presence eases over (the plan's clock slows into a pause rather than stopping dead).
const EASE: f32 = 0.04;
/// Seconds a new seq counts as the voice: its level comes a little after the phones the listener heard. Not in a silence
/// of over `HUSH` s, where a new seq is the listener hearing the room.
const SEQ_VOICE: f64 = 0.4;
/// The pace (words a second) from which the motion's timing follows it, the shortest wait before easing back, the share
/// of a word's time a glide may take at most, and the shortest glide.
const PACED_FROM: f32 = 0.5;
const MIN_WAIT: f32 = 0.3;
const GLIDE_SHARE: f32 = 0.6;
const MIN_GLIDE: f32 = 0.15;

/// Seconds of silence that make a stop, and seconds after the voice is back from one in which a step back is followed
/// at once.
const STOP: f64 = 0.3;
const RESUME: f64 = 2.0;

/// Seconds of silence after which, with plans and the voice held, nothing takes the pointer on until the voice is back:
/// the evidence of the last sounds before a pause comes within a block and the latency.
const HUSH: f64 = 1.0;
/// A held letter counts as the voice until this many times its expected time: a quiet microphone loses a ghunnah or a
/// madd to the floor.
const HOLD_VOICE: f32 = 1.5;
/// A Paused message sets the plan aside; Locked again within this long (s) with its seq, it goes on. Later the same seq
/// is no news: its flow is over.
const PAUSE_KEEP: f64 = 0.7;
/// A Tentative message this many words ahead of a place Locked within `EXCURSION_SECS` waits a block before it is
/// shown: mostly it is an excursion the next message takes back.
const EXCURSION: f32 = 2.0;
const EXCURSION_SECS: f64 = 0.5;
const BLOCK: f64 = 0.32;
/// The dim's time constants (s), going down and coming back.
const DIM_OUT: f32 = 0.35;
const DIM_IN: f32 = 0.06;

/// A plan being played: when its message came, and the seconds of its path played.
#[derive(Clone)]
struct Running {
    plan: Plan,
    at: f64,
    clock: f32,
    /// Seconds the voice has been absent in a row; once that reaches `expire` the rest of the plan is dropped.
    absent: f32,
    expired: bool,
    /// Where the plan itself has the reader: whether stale evidence still holds it inside the evidenced word is judged
    /// there, not by the glide trailing it.
    at_plan: f32,
    /// Its seq was played before (it comes again after a long pause, or after a message between): no news, and never
    /// past the drawn place or the evidenced letter.
    stale: bool,
}

impl Running {
    fn new(plan: &Plan, now: f64, stale: bool) -> Running {
        Running { plan: plan.clone(), at: now, clock: 0.0, absent: 0.0, expired: stale, at_plan: plan.now, stale }
    }

    /// Inside a held letter, until `HOLD_VOICE` times its expected time (a plan whose flow is over holds nothing).
    fn held(&self) -> bool {
        !self.expired && self.plan.hold.is_some_and(|(a, b)| self.clock >= a && self.clock <= a + HOLD_VOICE * (b - a))
    }

    /// The path still has flow ahead of the clock.
    fn flowing(&self) -> bool {
        !self.expired && self.plan.knots.last().is_some_and(|k| self.clock < k.0)
    }
}

/// A Tentative message waiting a block (`EXCURSION`).
#[derive(Clone)]
struct Pending {
    c: f32,
    conf: f32,
    speed: f32,
    t: f32,
    at: f64,
}

/// (Cloned to look ahead at where it will be drawn: `Marker` asks for its next frame then.)
#[derive(Default, Clone)]
pub struct Follower {
    target: Option<f32>,
    reading: Option<Reading>,
    conf: f32,
    /// Clock time (s) of the last update.
    recv: f64,
    pointer: Glider,
    start: Glider,
    end: Glider,
    /// Where the pointer, the start and the end were going on the last tick, and which of those goals
    /// stood still on it.
    goals: [f32; 3],
    steady: [bool; 3],
    /// Locked evidence of the last seconds as (position, audio time), for the reader's pace.
    seen: VecDeque<(f32, f32)>,
    pace: f32,
    behind_since: Option<f64>,
    /// How far it may run ahead without evidence: the end of the ayah being read (readers stop there).
    ayah_end: Option<f32>,
    /// Since when it has been parked at the end of the ayah just read (`Params::ayah_fade`): since the
    /// reader last sounded its last word.
    parked: Option<f64>,
    /// A plan has come: messages without one follow the rules that come with plans (`update_plan`).
    planned: bool,
    run: Option<Running>,
    /// The plan a Paused message set aside, and when; the last seq played.
    stashed: Option<(Running, f64)>,
    seq: Option<u32>,
    /// When the last new seq came.
    seq_at: f64,
    /// The second cap beside the ayah's end: the end of the line the evidence is on; and a respawn not drawn yet (put
    /// within it first).
    stop: Option<f32>,
    spawned: bool,
    /// News came in a long silence (`HUSH`) with the pointer here: it goes no further until the voice is back.
    held_at: Option<f32>,
    /// The voice as the link hears it (none: never heard of), and when the silence the reader is in began.
    level: Option<bool>,
    quiet_from: Option<f64>,
    /// When the voice came back from a stop (a silence of `STOP` s or more): a step back soon after is the reader starting
    /// again a word or two back, as one does after stopping where the meaning does not end.
    resumed_at: Option<f64>,
    /// How absent the voice is, eased over `EASE` (0: present); seconds of run-on it has allowed since the last
    /// message.
    hush: f32,
    voiced: f32,
    /// The pointer's goal on the last tick, for the feed-forward (none after news).
    goal_prev: Option<f32>,
    /// The last Locked place and when it came; a Tentative jump ahead of it waiting a block; the place an excursion
    /// left (a message back near it is followed at once); this message is such a return.
    locked: Option<(f32, f64)>,
    pending: Option<Pending>,
    excursion: Option<f32>,
    no_wait: bool,
    /// Where a paused pointer settles (where it is, or back at the evidenced word's end), and whether it has eased
    /// back there with `Params::uncross`: kept until the next message.
    settle: Option<f32>,
    retract: bool,
    /// When news last took the reader on, and the dim: when it last changed course, from and to how dark (1 - opacity).
    forward_at: f64,
    dark: (f64, f32, f32),
}

impl Follower {
    /// The furthest position the pointer may run ahead to before the reader is heard there: the end of
    /// the ayah being read (just inside its last word). `None` = no limit.
    pub fn set_ayah_end(&mut self, end: Option<f32>) {
        self.ayah_end = end;
    }

    /// The second cap beside the ayah's end: the end of the line the evidence is on, so the pointer changes line only
    /// once the reader is heard on the next, however far the plan's lead has gone. `None` = none.
    pub fn set_stop(&mut self, stop: Option<f32>) {
        self.stop = stop;
    }

    /// The link's news of the voice at clock time `now` (`presence::Heard`): present (sounding) or not, and whether the
    /// reader is in a silence.
    pub fn voice(&mut self, present: bool, quiet: bool, now: f64) {
        self.level = Some(present);
        if !quiet && self.quiet_from.is_some_and(|s| now - s >= STOP) {
            self.resumed_at = Some(now);
        }
        if quiet != self.quiet_from.is_some() {
            self.quiet_from = quiet.then_some(now);
        }
    }

    /// With the voice held, in a silence of over `HUSH` s outside a held letter of the plan being played.
    fn hushed(&self, now: f64, p: &Params) -> bool {
        p.voice && self.quiet_from.is_some_and(|s| now - s > HUSH) && !self.run.as_ref().is_some_and(|r| r.held() && self.reading == Some(Reading::Locked))
    }

    /// The word the evidence is on: the running plan's, else the last message's.
    pub fn evidence(&self) -> Option<u32> {
        self.run.as_ref().map(|r| r.plan.ev.0).or(self.target.map(|t| t.max(0.0) as u32))
    }

    /// "Follow tajweed timing" turned off: what came with plans is dropped, so the pointer moves as one that never saw a
    /// plan; a Tentative message waiting its block is shown now.
    fn forget_plans(&mut self, now: f64, p: &Params) {
        let pending = self.pending.take();
        (self.planned, self.run, self.stashed, self.seq, self.locked, self.excursion, self.no_wait) = (false, None, None, None, None, None, false);
        (self.settle, self.retract, self.goal_prev, self.held_at) = (None, false, None, None);
        if let Some(q) = pending {
            self.place(q.c, Reading::Tentative, q.conf, q.speed, q.t, now, p);
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update(&mut self, c: f32, reading: Reading, conf: f32, speed_units: f32, t_audio: f32, now: f64, p: &Params) -> Jump {
        let p = &self.paced(p);
        if self.planned && !p.plan {
            self.forget_plans(now, p);
        }
        if self.planned {
            self.pending = None;
            let near = self.locked.is_some_and(|(l, at)| now - at < EXCURSION_SECS && c >= l + EXCURSION);
            if reading == Reading::Tentative && near && self.parked.is_none() && self.target.is_some() && c <= self.pointer.x + p.far_jump {
                self.pending = Some(Pending { c, conf, speed: speed_units, t: t_audio, at: now });
                return Jump::Glide;
            }
            self.unplan(reading, now);
        }
        self.place(c, reading, conf, speed_units, t_audio, now, p)
    }

    /// A message with a plan: its place is `plan.now`, under `update`'s rules for jumps. The same seq again changes only
    /// the state and the confidence: the running plan and its clock go on, or, after a long pause or a message between,
    /// its plan is no news (its clock does not start, nothing past the drawn place or the evidenced letter, no undim).
    #[allow(clippy::too_many_arguments)]
    pub fn update_plan(&mut self, plan: &Plan, reading: Reading, conf: f32, speed_units: f32, t_audio: f32, now: f64, p: &Params) -> Jump {
        let p = &self.paced(p);
        self.planned = true;
        self.pending = None;
        if let Some((mut run, at)) = self.stashed.take()
            && run.plan.seq == plan.seq
        {
            if now - at >= PAUSE_KEEP {
                (run.expired, run.stale) = (true, true);
            }
            self.run = Some(run);
        }
        if self.target.is_some() && (self.run.as_ref().is_some_and(|r| r.plan.seq == plan.seq) || self.seq == Some(plan.seq)) {
            if self.run.as_ref().is_none_or(|r| r.plan.seq != plan.seq) {
                self.run = Some(Running::new(plan, now, true));
            }
            // Still sounding the ayah's last word (a long madd): its pause starts again.
            if self.parked.is_some() && reading != Reading::Paused {
                self.parked = Some(now);
            }
            (self.reading, self.conf) = (Some(reading), conf);
            return Jump::Glide;
        }
        let jump = self.place(plan.now, reading, conf, speed_units, t_audio, now, p);
        self.run = Some(Running::new(plan, now, false));
        (self.seq, self.seq_at, self.forward_at) = (Some(plan.seq), now, now);
        jump
    }

    /// A message without a plan ends the running one; a Paused message sets it aside for a moment.
    fn unplan(&mut self, reading: Reading, now: f64) {
        if let Some(run) = self.run.take()
            && reading == Reading::Paused
        {
            self.stashed = Some((run, now));
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn place(&mut self, c: f32, reading: Reading, conf: f32, speed_units: f32, t_audio: f32, now: f64, p: &Params) -> Jump {
        let x = self.pointer.x;
        let last = self.seen.back().map(|s| s.0);
        let went_back = self.target.is_some() && last.is_some_and(|l| c < l - p.fade_back) && c < x - p.fade_back;
        let mut jump = if self.target.is_none() || went_back || c > x + p.far_jump { Jump::Respawn } else { Jump::Glide };
        // The ayah's last word (`ayah_end` is just inside it, from the update before).
        let in_last_word = self.ayah_end.is_some_and(|e| c >= e - 1.0 && c <= e + 0.05);
        if self.parked.is_some() && !in_last_word {
            jump = Jump::Respawn;
        }
        let parking = in_last_word && (self.parked.is_some() || (p.ayah_fade.is_some() && reading == Reading::Paused));
        // The pause is counted from the last time the reader was heard sounding the word.
        self.parked = match (parking, reading) {
            (false, _) => None,
            (true, Reading::Paused) => self.parked.or(Some(now)),
            (true, _) => Some(now),
        };
        if jump == Jump::Respawn {
            let half = uncertainty(reading, conf, 0.0) * p.spread;
            self.pointer.snap(c, now);
            self.start.snap(c - half, now);
            self.end.snap(c + half, now);
            self.seen.clear();
            self.behind_since = None;
            self.spawned = true;
        } else {
            for part in [&mut self.pointer, &mut self.start, &mut self.end] {
                part.retarget(now);
            }
        }
        if reading == Reading::Locked {
            self.seen.retain(|&(sc, st)| t_audio - st < 4.0 && sc <= c + 0.01);
            self.seen.push_back((c, t_audio));
        }
        let span = match (self.seen.front(), self.seen.back()) {
            (Some(a), Some(b)) => b.1 - a.1,
            _ => 0.0,
        };
        let measured = (span >= 1.5).then(|| (self.seen.back().unwrap().0 - self.seen.front().unwrap().0).max(0.0) / span);
        let estimate = measured.unwrap_or(if speed_units > 0.0 { speed_units / UNITS_PER_WORD } else { self.pace });
        self.pace += (estimate.min(2.5) - self.pace) * 0.35;
        if self.target.is_none_or(|t| c > t + 1e-4) {
            self.forward_at = now;
        }
        if jump == Jump::Respawn {
            self.held_at = None;
        }
        if self.planned && self.hushed(now, p) {
            self.held_at.get_or_insert(self.pointer.x);
        }
        let mut no_wait = false;
        if self.planned {
            // Back near where a Tentative excursion left from: followed at once, without the wait. The excursion is
            // remembered while Tentative messages keep it out.
            let left = self.excursion.take();
            no_wait = left.is_some_and(|e| (c - e).abs() <= 1.0 && c < x);
            if reading == Reading::Tentative && left.is_some_and(|e| c > e + 1.0) {
                self.excursion = left;
            }
            if reading == Reading::Locked {
                self.locked = Some((c, now));
            }
        }
        // A step back as the reader goes on from a stop is them starting again there: followed at once, not waited out.
        self.no_wait = no_wait || (self.resumed_at.is_some_and(|r| now - r < RESUME) && c < x);
        self.target = Some(c);
        self.reading = Some(reading);
        self.conf = conf;
        self.recv = now;
        (self.voiced, self.goal_prev, self.settle, self.retract) = (0.0, None, None, false);
        jump
    }

    /// The motion's timing at the reader's pace: the wait before easing back about a word's time, and a glide no longer
    /// than most of one, so a fast reader is caught up with and a step back comes soon. With movement set by hand, below
    /// `PACED_FROM` words a second, or before a pace is known, the settings as they are.
    fn paced(&self, p: &Params) -> Params {
        if !p.paced || self.pace < PACED_FROM {
            return *p;
        }
        let word = 1.0 / self.pace;
        let mut q = *p;
        q.wait = p.wait.min(word).max(MIN_WAIT.min(p.wait));
        for e in [&mut q.pointer, &mut q.start, &mut q.end] {
            e.secs = e.secs.min(GLIDE_SHARE * word).max(MIN_GLIDE.min(e.secs));
        }
        q
    }

    /// The place last followed (where the pointer is going), if it shows.
    pub fn target(&self) -> Option<f32> {
        self.target
    }

    /// The reader's pace as measured lately, words a second.
    pub fn pace(&self) -> f32 {
        self.pace
    }

    /// The tracker lost the place (searching, or the word is not on this page).
    pub fn hide(&mut self) {
        self.target = None;
        self.parked = None;
        (self.run, self.stashed, self.pending, self.excursion) = (None, None, None, None);
    }

    /// The opacity the pointer is drawn at, times the view's fades: after the delay without the voice or news of the
    /// reader going on, outside a held letter and not parked, it eases down to `quiet_dim`, and back when either
    /// returns.
    pub fn dim(&self, now: f64) -> f32 {
        let (t0, from, to) = self.dark;
        let tau = if to > from { DIM_OUT } else { DIM_IN };
        1.0 - (to + (from - to) * (-((now - t0).max(0.0) as f32) / tau).exp())
    }

    /// A plan's flow plays: its clock runs with no news from the listener (and what it holds or lets go with it).
    pub fn flowing(&self) -> bool {
        self.run.as_ref().is_some_and(|r| r.flowing() && self.reading == Some(Reading::Locked))
    }

    /// Whether the voice is present: always without news of it or with `Params::voice` off; while a new seq is fresh, but
    /// not in a long silence; and inside a held letter of the plan being played.
    fn present(&self, now: f64, p: &Params) -> bool {
        self.level(p)
            || (now - self.seq_at < SEQ_VOICE && !self.hushed(now, p))
            || self.run.as_ref().is_some_and(|r| r.held() && self.reading == Some(Reading::Locked))
    }

    /// The voice's presence by its level alone.
    fn level(&self, p: &Params) -> bool {
        !p.voice || self.level.is_none_or(|present| present)
    }

    /// When the dim will start with no news: the delay after both the silence (by the level, whether or not
    /// `Params::voice` holds the run-on) and the news of the reader going on began, outside a held letter and not parked.
    fn dims_at(&self, p: &Params) -> Option<f64> {
        let since = self.quiet_from?;
        let quiet = p.quiet_dim < 1.0 && self.parked.is_none() && !self.run.as_ref().is_some_and(Running::held);
        quiet.then(|| since.max(self.forward_at) + f64::from(p.quiet_delay))
    }

    /// The plan's goal at `now`, `dt` after the last tick: its clock runs while the place is Locked, eased by the
    /// voice's presence, and stops for good once the voice has been gone `expire` (or, strict, at a held letter's
    /// expected end); the mean of the plan around it, never past the evidenced letter by more than `max_ahead`, with
    /// stale evidence not past the evidenced word, and with a seq played before not past the drawn place or the
    /// evidenced letter.
    fn plan_goal(&mut self, now: f64, dt: f32, present: bool, p: &Params) -> Option<f32> {
        let (hush, x) = (self.hush, self.pointer.x);
        let run = self.run.as_mut().filter(|_| self.reading == Some(Reading::Locked))?;
        run.absent = if present { 0.0 } else { run.absent + dt };
        run.expired |= run.absent >= p.expire;
        if !run.expired {
            run.clock += dt * (1.0 - hush);
        }
        if p.strict
            && let Some((_, b)) = run.plan.hold
        {
            run.clock = run.clock.min(b);
        }
        let h = p.smooth / 2.0;
        let cap = run.plan.ev_end + p.max_ahead;
        let mut goal = ([-h, -h / 2.0, 0.0, h / 2.0, h].iter().map(|o| run.plan.position(run.clock + o)).sum::<f32>() / 5.0).min(cap);
        let mut at_plan = run.plan.position(run.clock).min(cap);
        let ev = run.plan.ev.0 as f32;
        if p.fresh > 0.0 && run.plan.age + (now - run.at) as f32 > p.fresh && run.at_plan < ev + 0.96 {
            goal = goal.min(ev + 0.95);
            at_plan = at_plan.min(ev + 0.95);
        }
        if run.stale {
            goal = goal.min(x.max(run.plan.ev_end));
        }
        run.at_plan = at_plan;
        Some(goal)
    }

    /// Sets the dim's course at `now` (`dims_at`), back up as soon as it no longer holds.
    fn darken(&mut self, now: f64, p: &Params) {
        let to = if self.dims_at(p).is_some_and(|at| now >= at) { 1.0 - p.quiet_dim.max(0.0) } else { 0.0 };
        if to != self.dark.2 {
            self.dark = (now, 1.0 - self.dim(now), to);
        }
    }

    /// Parked at the end of the ayah just read for at least the delay: the view fades the pointer out
    /// until the next update elsewhere (`Jump::Respawn` there).
    pub fn faded(&self, now: f64, p: &Params) -> bool {
        matches!((self.parked, p.ayah_fade), (Some(since), Some(delay)) if now - since >= f64::from(delay))
    }

    /// The pointer or an edge of the range moved on the last tick, or is still on its way.
    #[cfg(test)]
    pub fn moving(&self) -> bool {
        let parts = [&self.pointer, &self.start, &self.end];
        self.steady != [true; 3] || parts.iter().zip(self.goals).any(|(part, goal)| part.moving(goal))
    }

    /// Puts the pointer and each edge of the range at its goal once it is less than `within` words from it (`within`
    /// for each: half a pixel there) with its goal still on the last tick: the rest of its glide would never show. Not
    /// while a plan plays (its clock moves the goals). Which of them are at rest.
    pub fn settle(&mut self, within: [f32; 3], now: f64, p: &Params) -> [bool; 3] {
        let flowing = self.flowing();
        let eases = [p.pointer, p.start, p.end];
        let parts = [&mut self.pointer, &mut self.start, &mut self.end];
        let mut rest = [false; 3];
        for (k, part) in parts.into_iter().enumerate() {
            rest[k] = !flowing && self.steady[k] && part.left(self.goals[k], eases[k]) < within[k];
            if rest[k] {
                part.snap(self.goals[k], now);
            }
        }
        rest
    }

    /// Seconds until something changes with no news from the listener: the end of the wait for a
    /// reader who is behind, or of the pause before a parked pointer fades out.
    pub fn wake(&self, now: f64, p: &Params) -> Option<f32> {
        let p = &self.paced(p);
        // In f64 as `faded` and `tick` count: a frame a hair early must still ask for the next one.
        let left = |since: f64, secs: f32| {
            let left = since + f64::from(secs) - now;
            (left > 0.0).then_some(left as f32)
        };
        let wait = self.behind_since.filter(|_| self.reading != Some(Reading::Paused) && !self.no_wait).and_then(|since| left(since, p.wait));
        let fade = self.parked.zip(p.ayah_fade).and_then(|(since, delay)| left(since, delay));
        // A Tentative jump waiting its block, and the dim's start.
        let pending = self.pending.as_ref().and_then(|q| left(q.at, BLOCK as f32));
        let dims = self.dims_at(p).and_then(|at| left(at, 0.0));
        [wait, fade, pending, dims].into_iter().flatten().reduce(f32::min)
    }

    fn goal(&self, target: f32, now: f64, p: &Params) -> f32 {
        // The run-on counts only while the voice is present, once there is news of it.
        let dt = if p.voice && self.level.is_some() { self.voiced.min(RUN_ON) } else { ((now - self.recv) as f32).clamp(0.0, RUN_ON) };
        let v = match self.reading {
            Some(Reading::Paused) => 0.0,
            Some(Reading::Tentative) => self.pace * 0.5,
            // With plans, Drifting is the listener's own dead reckoning: nothing on top of it.
            Some(Reading::Drifting) if self.planned => 0.0,
            _ => self.pace,
        };
        target + (v * dt).min(p.max_ahead)
    }

    /// Advances the pointer and the range's edges to clock time `now` (s).
    pub fn tick(&mut self, now: f64, dt: f32, p: &Params) -> Option<Drawn> {
        let p = &self.paced(p);
        if self.planned && !p.plan {
            self.forget_plans(now, p);
        }
        if let Some(q) = self.pending.take_if(|q| now - q.at >= BLOCK) {
            self.excursion = self.locked.map(|l| l.0);
            self.unplan(Reading::Tentative, now);
            // Ahead and near (a glide): the view has nothing to fade over.
            self.place(q.c, Reading::Tentative, q.conf, q.speed, q.t, now, p);
        }
        let target = self.target?;
        let reading = self.reading?;
        let present = self.present(now, p);
        if p.voice && self.level.is_some() {
            self.hush += (f32::from(u8::from(!present)) - self.hush) * (1.0 - (-dt / EASE).exp());
            self.voiced += dt * (1.0 - self.hush);
        }
        // Where the reader is heard: the plan, or the place with the run-ahead.
        let planned = self.plan_goal(now, dt, present, p);
        let base = self.run.as_ref().filter(|_| planned.is_some()).map_or(target, |r| r.plan.now);
        let mut est = planned.unwrap_or_else(|| self.goal(target, now, p));
        if let Some(end) = self.ayah_end {
            est = est.min(end.max(base));
        }
        let half = uncertainty(reading, self.conf, (now - self.recv) as f32) * p.spread;
        if let Some(stop) = self.stop {
            est = est.min(stop);
            if self.spawned && self.pointer.x > stop {
                self.pointer.snap(stop, now);
                self.start.snap(stop - half, now);
                self.end.snap(stop + half, now);
            }
        }
        self.spawned = false;
        // With plans, news in a long silence takes the pointer no further until the voice is back: the listener's steps
        // there are the room's.
        if let Some(h) = self.held_at {
            if self.quiet_from.is_some() {
                est = est.min(h);
            } else {
                // Let go, the goal is news: not the plan's own motion to take at once.
                (self.held_at, self.goal_prev) = (None, None);
            }
        }
        let mut goal = est;
        let x = self.pointer.x;
        // The evidenced word: the plan's, else the message's.
        let ev = self.run.as_ref().map_or(target.floor(), |r| r.plan.ev.0 as f32);
        let word = (x + 1e-6).floor();
        self.retract |= self.planned && p.uncross && !self.level(p) && word > ev;
        if self.retract {
            goal = goal.min(ev + 0.95);
            est = est.min(goal);
            self.behind_since = None;
        } else if self.planned && reading == Reading::Paused && (goal < x || self.settle.is_some()) {
            // A paused reader: nothing goes back inside the evidenced word (a held letter being crept is in it), nor
            // from just past its end; from further on, back to that end.
            let rest = word <= ev || x - (ev + 1.0) <= p.rest;
            goal = *self.settle.get_or_insert(if rest { x } else { ev + 0.95 });
        } else if est < x {
            // Ahead of the reader: wait for them (the evidence comes in bursts), and only then ease
            // back; a reader who has stopped is not coming, so then at once.
            let since = *self.behind_since.get_or_insert(now);
            if now - since < f64::from(p.wait) && reading != Reading::Paused && !self.no_wait {
                goal = x;
            }
        } else {
            self.behind_since = None;
        }
        if self.planned {
            // The plan's own forward motion is taken at once in part: a glide alone trails a moving goal.
            if let Some(gp) = self.goal_prev
                && goal > gp
                && f64::from(goal) >= self.pointer.at()
            {
                self.pointer.advance((goal - gp) * p.ff, goal);
            }
            self.goal_prev = Some(goal);
        }
        // The start reaches back to where the reader is heard even while the pointer waits; the end
        // stays ahead of the pointer.
        let goals = [goal, est - half, goal + half];
        // While plans play the pointer's place is kept to the last bit (`Glider::step_exact`).
        let x = if self.planned { self.pointer.step_exact(goals[0], now, dt, p.pointer) } else { self.pointer.step(goals[0], now, dt, p.pointer) };
        let start = self.start.step(goals[1], now, dt, p.start).min(x);
        let end = self.end.step(goals[2], now, dt, p.end).max(x);
        self.steady = [0, 1, 2].map(|k| goals[k] == self.goals[k]);
        self.goals = goals;
        self.darken(now, p);
        Some(Drawn { x, start, end })
    }
}

/// How unsure the tracker is, in words: tight when Locked and confident, wider when Tentative, and
/// growing while it drifts on without evidence.
pub fn uncertainty(reading: Reading, conf: f32, since: f32) -> f32 {
    match reading {
        Reading::Locked => 0.45 + 0.8 * (1.0 - conf.clamp(0.0, 1.0)),
        Reading::Paused => 0.55,
        Reading::Drifting => (1.0 + 0.8 * since).min(4.0),
        Reading::Tentative => 1.6,
    }
}

#[cfg(test)]
mod tests {
    use quran_core::settings::VoiceCurve as Curve;

    use super::super::glide::CURVES;
    use super::*;

    const FRAME: f64 = 1.0 / 60.0;

    /// Ticks at 60 frames a second from `from` for `secs`; returns where everything was drawn last.
    fn run(f: &mut Follower, from: f64, secs: f64, p: &Params) -> Drawn {
        let mut t = from;
        let mut d = None;
        while t < from + secs {
            t += FRAME;
            d = f.tick(t, FRAME as f32, p);
        }
        d.unwrap()
    }

    fn all(curve: Curve) -> Params {
        let e = Ease { curve, secs: 0.32 };
        Params { pointer: e, start: e, end: e, ..Default::default() }
    }

    #[test]
    fn every_curve_glides_to_a_held_word() {
        for (curve, _) in CURVES {
            let p = all(curve);
            let mut f = Follower::default();
            assert_eq!(f.update(3.5, Reading::Locked, 1.0, 0.0, 0.0, 0.0, &p), Jump::Respawn);
            f.update(5.5, Reading::Paused, 1.0, 0.0, 0.3, 0.3, &p);
            let d = run(&mut f, 0.3, 3.0, &p);
            let half = uncertainty(Reading::Paused, 1.0, 0.0) * p.spread;
            assert!((d.x - 5.5).abs() < 0.05, "{curve:?} ended at {}", d.x);
            assert!((d.start - (5.5 - half)).abs() < 0.05 && (d.end - (5.5 + half)).abs() < 0.05, "{curve:?}: {d:?}");
        }
    }

    #[test]
    fn the_range_holds_the_pointer_whatever_the_curves() {
        for (a, _) in CURVES {
            for (b, _) in CURVES {
                for (c, _) in CURVES {
                    let p = Params {
                        pointer: Ease { curve: a, secs: 0.32 },
                        start: Ease { curve: b, secs: 0.9 },
                        end: Ease { curve: c, secs: 0.1 },
                        ..Default::default()
                    };
                    let mut f = Follower::default();
                    let mut t = 0.0;
                    // Bursts: a word held, then two at once, a step back, a pause.
                    for (k, c) in [10.0, 10.2, 10.2, 12.1, 12.3, 11.8, 12.6, 14.0, 14.0, 14.1].into_iter().enumerate() {
                        let reading = if k == 8 { Reading::Paused } else { Reading::Locked };
                        f.update(c, reading, 0.9, 6.0, t as f32, t, &p);
                        while t < (k + 1) as f64 * 0.32 {
                            t += FRAME;
                            let d = f.tick(t, FRAME as f32, &p).unwrap();
                            assert!(d.start <= d.x && d.x <= d.end, "{a:?}/{b:?}/{c:?} at {t}: {d:?}");
                        }
                    }
                }
            }
        }
        // Equal settings: the range is centred on a pointer at rest.
        let p = Params::default();
        let mut f = Follower::default();
        f.update(7.3, Reading::Locked, 0.8, 0.0, 0.0, 0.0, &p);
        let d = run(&mut f, 0.0, 3.0, &p);
        let half = uncertainty(Reading::Locked, 0.8, 3.0) * p.spread;
        assert!(((d.end - d.x) - half).abs() < 1e-3 && ((d.x - d.start) - half).abs() < 1e-3, "{d:?}");
    }

    #[test]
    fn a_slow_start_trails_further_behind_a_reader() {
        let behind = |ms: f32| {
            let p = Params { start: Ease { curve: Curve::Smooth, secs: ms / 1000.0 }, ..Default::default() };
            let mut f = Follower::default();
            let mut d = None;
            for k in 0..20 {
                let t = f64::from(k) * 0.32;
                f.update(10.0 + k as f32 * 0.4, Reading::Locked, 0.9, 7.6, t as f32, t, &p);
                d = Some(run(&mut f, t, 0.32, &p));
            }
            let d = d.unwrap();
            d.x - d.start
        };
        assert!(behind(1500.0) > behind(60.0) + 0.3, "{} vs {}", behind(1500.0), behind(60.0));
    }

    #[test]
    fn runs_on_at_pace_but_not_past_max_ahead() {
        let p = Params::default();
        let mut f = Follower::default();
        for k in 0..10 {
            f.update(k as f32 * 0.5, Reading::Locked, 1.0, 0.0, k as f32 * 0.32, k as f64 * 0.32, &p);
        }
        let x = run(&mut f, 9.0 * 0.32, 3.0, &p).x;
        assert!(x <= 4.5 + p.max_ahead + 1e-3 && x > 4.5, "x {x}");
    }

    #[test]
    fn running_ahead_keeps_frames_coming_until_max_ahead() {
        // Linear, time-based, sits on its goal once its glide is over; the goal still runs on.
        let p = all(Curve::Linear);
        let mut f = Follower::default();
        for k in 0..10 {
            f.update(k as f32 * 0.4, Reading::Locked, 1.0, 0.0, k as f32 * 0.32, f64::from(k) * 0.32, &p);
        }
        let t = 9.0 * 0.32;
        run(&mut f, t, 0.35, &p);
        assert!(f.moving(), "still running ahead");
        run(&mut f, t + 0.35, 1.0, &p);
        assert!(!f.moving(), "at max_ahead");
    }

    #[test]
    fn a_small_step_back_waits_then_eases_back() {
        let p = Params { wait: 1.0, ..Default::default() };
        let mut f = Follower::default();
        f.update(10.0, Reading::Locked, 1.0, 0.0, 0.0, 0.0, &p);
        f.update(10.0, Reading::Paused, 1.0, 0.0, 0.3, 0.3, &p);
        run(&mut f, 0.3, 1.0, &p);
        assert_eq!(f.update(9.4, Reading::Locked, 1.0, 0.0, 1.3, 1.3, &p), Jump::Glide);
        let held = run(&mut f, 1.3, 0.8, &p).x;
        assert!((held - 10.0).abs() < 1e-3, "should wait, at {held}");
        let back = run(&mut f, 2.1, 2.0, &p).x;
        assert!((back - 9.4).abs() < 0.2, "should ease back, at {back}");
    }

    #[test]
    fn a_step_back_as_the_reader_goes_on_from_a_stop_is_followed_at_once() {
        // Word 10 read, a stop (no voice from 0.4 s to 1.2 s), then the reader starts again a word back.
        let p = Params { wait: 1.0, voice: true, ..Default::default() };
        let mut f = Follower::default();
        f.update(10.0, Reading::Locked, 1.0, 0.0, 0.0, 0.0, &p);
        f.voice(false, true, 0.4);
        run(&mut f, 0.0, 1.2, &p);
        f.voice(true, false, 1.2);
        assert_eq!(f.update(9.4, Reading::Locked, 1.0, 0.0, 1.3, 1.3, &p), Jump::Glide);
        let back = run(&mut f, 1.3, 0.8, &p).x;
        assert!(back < 9.7, "should not wait, at {back}");
        // Long after the voice came back, a step back is waited out again (it may be jitter).
        f.update(10.0, Reading::Locked, 1.0, 0.0, 4.0, 4.0, &p);
        run(&mut f, 4.0, 1.0, &p);
        f.update(9.4, Reading::Locked, 1.0, 0.0, 5.0, 5.0, &p);
        let held = run(&mut f, 5.0, 0.6, &p).x;
        assert!((held - 10.0).abs() < 0.05, "should wait, at {held}");
    }

    #[test]
    fn at_a_fast_pace_a_step_back_waits_about_a_word_and_glides_are_short() {
        // Words reported every 0.4 s at 12 units a second (the listener's speed: about 2.5 words a second).
        let p = Params { wait: 1.0, paced: true, ..Default::default() };
        let mut f = Follower::default();
        for k in 0..8 {
            let t = f64::from(k) * 0.4;
            f.update(10.0 + k as f32, Reading::Locked, 1.0, 12.0, t as f32, t, &p);
            run(&mut f, t, 0.4, &p);
        }
        let q = f.paced(&p);
        assert!(q.wait < 0.5 && q.wait >= MIN_WAIT, "wait {}", q.wait);
        assert!(q.pointer.secs <= 0.6 / f.pace + 1e-6, "glide {}", q.pointer.secs);
        // A step back now is followed after about a word's time, not the second the settings say.
        let before = run(&mut f, 3.2 - FRAME, FRAME, &p).x;
        f.update(before - 1.0, Reading::Locked, 1.0, 12.0, 3.2, 3.2, &p);
        let back = run(&mut f, 3.2, 0.9, &p).x;
        assert!(back < before - 0.3, "should have eased back from {before}, at {back}");
        // No pace known, or movement set by hand: the settings as they are.
        assert_eq!(Follower::default().paced(&p), p);
        let manual = Params { paced: false, ..p };
        assert_eq!(f.paced(&manual), manual);
    }

    #[test]
    fn while_the_pointer_waits_the_range_reaches_back_to_the_reader() {
        let p = Params { wait: 1.0, ..Default::default() };
        let half = uncertainty(Reading::Locked, 1.0, 0.0) * p.spread;
        let start = || {
            let mut f = Follower::default();
            f.update(10.0, Reading::Locked, 1.0, 0.0, 0.0, 0.0, &p);
            let before = run(&mut f, 0.0, 1.0, &p);
            assert_eq!(f.update(9.4, Reading::Locked, 1.0, 0.0, 1.0, 1.0, &p), Jump::Glide, "within the waiting range");
            (f, before)
        };
        let (mut f, before) = start();
        let held = run(&mut f, 1.0, 0.9, &p);
        assert!((held.x - 10.0).abs() < 1e-3, "the pointer waits: {held:?}");
        assert!((held.start - (9.4 - half)).abs() < 0.05, "the start reaches back to the reader: {held:?}");
        assert!((held.end - before.end).abs() < 0.05, "the end stays: {held:?} from {before:?}");
        // The wait is over, the reader still behind: the pointer and the end come back too.
        let back = run(&mut f, 1.9, 2.5, &p);
        assert!((back.x - 9.4).abs() < 0.05 && (back.end - (9.4 + half)).abs() < 0.05, "{back:?}");
        // A reader who goes on again within the wait brings the start forward with them.
        let (mut f, _) = start();
        run(&mut f, 1.0, 0.5, &p);
        f.update(10.3, Reading::Locked, 1.0, 0.0, 1.5, 1.5, &p);
        let on = run(&mut f, 1.5, 1.5, &p);
        assert!((on.start - (10.3 - half)).abs() < 0.05 && (on.x - 10.3).abs() < 0.05, "{on:?}");
    }

    #[test]
    fn a_paused_reader_is_not_waited_for_and_the_ayah_end_holds() {
        let p = Params::default();
        let mut f = Follower::default();
        f.update(10.0, Reading::Locked, 1.0, 0.0, 0.0, 0.0, &p);
        run(&mut f, 0.0, 1.0, &p);
        f.update(9.6, Reading::Paused, 1.0, 0.0, 1.0, 1.0, &p);
        assert!((run(&mut f, 1.0, 1.0, &p).x - 9.6).abs() < 0.05);
        let mut g = Follower::default();
        for k in 0..8 {
            g.update(5.0 + k as f32, Reading::Locked, 1.0, 0.0, k as f32, f64::from(k), &p);
            g.set_ayah_end(Some(12.95));
        }
        assert!(run(&mut g, 7.0, 3.0, &p).x <= 12.95 + 1e-3);
    }

    #[test]
    fn going_back_far_or_jumping_far_ahead_respawns() {
        let p = Params::default();
        let mut f = Follower::default();
        f.update(20.0, Reading::Locked, 1.0, 0.0, 0.0, 0.0, &p);
        run(&mut f, 0.0, 0.5, &p);
        assert_eq!(f.update(17.0, Reading::Locked, 1.0, 0.0, 0.5, 0.5, &p), Jump::Respawn);
        // Everything is at the new place at once.
        let half = uncertainty(Reading::Locked, 1.0, 0.0) * p.spread;
        let d = f.tick(0.5, 0.0, &p).unwrap();
        assert!((d.x - 17.0).abs() < 1e-4 && (d.start - (17.0 - half)).abs() < 1e-4 && (d.end - (17.0 + half)).abs() < 1e-4, "{d:?}");
        assert_eq!(f.update(17.0 + p.far_jump + 1.0, Reading::Locked, 1.0, 0.0, 0.8, 0.8, &p), Jump::Respawn);
        assert_eq!(f.update(31.0, Reading::Locked, 1.0, 0.0, 1.1, 1.1, &p), Jump::Glide);
    }

    #[test]
    fn uncertainty_grows_while_drifting() {
        assert!(uncertainty(Reading::Locked, 1.0, 0.0) < uncertainty(Reading::Tentative, 1.0, 0.0));
        assert!(uncertainty(Reading::Drifting, 1.0, 2.0) > uncertainty(Reading::Drifting, 1.0, 0.5));
        assert!(uncertainty(Reading::Drifting, 1.0, 99.0) <= 4.0);
    }

    /// A follower reading an ayah whose last word is word 10, paused in it at 0.5 s.
    fn parked(p: &Params) -> Follower {
        let mut f = Follower::default();
        f.update(10.2, Reading::Locked, 0.9, 6.0, 0.0, 0.0, p);
        f.set_ayah_end(Some(10.95));
        run(&mut f, 0.0, 0.5, p);
        assert_eq!(f.update(10.6, Reading::Paused, 0.9, 6.0, 0.5, 0.5, p), Jump::Glide);
        f
    }

    #[test]
    fn a_pause_at_the_ayah_end_fades_after_the_delay_until_the_reader_goes_on() {
        let p = Params { ayah_fade: Some(1.0), ..Default::default() };
        for (next, why) in [(11.1, "the next ayah"), (8.3, "a step back"), (40.0, "another ayah")] {
            let mut f = parked(&p);
            assert!(!f.faded(1.4, &p) && f.faded(1.5, &p), "after the delay");
            assert_eq!(f.wake(1.0, &p), Some(0.5));
            // Still sounding the last word (a long madd): the pause starts again.
            f.update(10.8, Reading::Locked, 0.9, 6.0, 1.2, 1.2, &p);
            assert!(!f.faded(2.1, &p) && f.faded(2.2, &p), "more of the last word keeps it parked");
            f.update(10.8, Reading::Paused, 0.9, 6.0, 1.5, 1.5, &p);
            assert!(f.faded(2.2, &p), "a pause goes on counting");
            assert_eq!(f.update(next, Reading::Locked, 0.9, 6.0, 2.5, 2.5, &p), Jump::Respawn, "{why}");
            assert!(!f.faded(9.0, &p));
        }
        // Going on before the delay is up: still a respawn.
        let mut f = parked(&p);
        assert_eq!(f.update(11.1, Reading::Locked, 0.9, 6.0, 0.8, 0.8, &p), Jump::Respawn);
        let now = Params { ayah_fade: Some(0.0), ..p };
        assert!(parked(&now).faded(0.5, &now), "no delay: at once");
        // A frame a hair before the end of the pause still asks for the next.
        let three = Params { ayah_fade: Some(3.0), ..p };
        let near = 0.5 + 3.0 - 5e-8;
        assert!(!parked(&three).faded(near, &three) && parked(&three).wake(near, &three).is_some());
        let off = Params { ayah_fade: None, ..p };
        let f = parked(&off);
        assert!(!f.faded(9.0, &off) && f.wake(9.0, &off).is_none(), "off: never");
    }

    #[test]
    fn at_rest_it_asks_for_no_frames_and_wakes_at_the_end_of_a_wait() {
        let p = Params { wait: 4.0, ..Default::default() };
        let mut f = Follower::default();
        f.update(10.0, Reading::Locked, 1.0, 6.0, 0.0, 0.0, &p);
        run(&mut f, 0.0, 0.3, &p);
        f.update(10.4, Reading::Paused, 1.0, 6.0, 0.3, 0.3, &p);
        run(&mut f, 0.3, 3.0, &p);
        assert!(!f.moving() && f.wake(3.3, &p).is_none(), "settled after a pause");
        // A step back: the pointer holds for the wait (the range settles meanwhile), and wakes when it
        // ends.
        f.update(10.0, Reading::Locked, 1.0, 0.0, 3.3, 3.3, &p);
        let d = run(&mut f, 3.3, 3.0, &p);
        assert!((d.x - 10.4).abs() < 0.01 && !f.moving(), "{d:?}");
        let left = f.wake(6.3, &p).unwrap();
        assert!((left - 1.0).abs() < 0.05, "{left}");
        run(&mut f, 6.3, 1.1, &p);
        assert!(f.moving(), "eases back once the wait is over");
    }

    #[test]
    fn frames_far_apart_or_uneven_draw_what_steady_frames_do() {
        // A word on, a step back waited out, a pause: ticked at steady 60 a second, or unevenly with a tick when a wait
        // ends (as the marker's frames come: when asked, a frame after news, and at the follower's own wakes), the
        // pointer and the range's edges are in the same places at the same moments.
        let p = Params::default();
        let news = [(0.0, 10.0, Reading::Locked), (0.32, 10.4, Reading::Locked), (0.64, 10.2, Reading::Locked), (2.5, 10.6, Reading::Paused)];
        let marks = [0.3, 0.6, 1.0, 1.6, 2.13, 2.2, 2.4, 3.0, 4.0];
        let play = |steps: &[f64], at_wakes: bool| {
            let mut f = Follower::default();
            let (mut t, mut k, mut seen, mut next, mut heard) = (0.0f64, 0, Vec::new(), 0, true);
            f.update(news[0].1, news[0].2, 1.0, 0.0, 0.0, 0.0, &p);
            next += 1;
            for &mark in &marks {
                while t < mark - 1e-9 {
                    let step = if std::mem::take(&mut heard) { FRAME } else { steps[k % steps.len()] };
                    let mut to = (t + step).min(mark).min(news.get(next).map_or(f64::INFINITY, |n| n.0));
                    if at_wakes && let Some(w) = f.wake(t, &p) {
                        to = to.min(t + f64::from(w));
                    }
                    let d = f.tick(to, (to - t) as f32, &p).unwrap();
                    (t, k) = (to, k + 1);
                    if let Some(&(at, c, r)) = news.get(next).filter(|n| n.0 <= t + 1e-9) {
                        f.update(c, r, 1.0, 0.0, at as f32, at, &p);
                        (next, heard) = (next + 1, true);
                    }
                    if (t - mark).abs() < 1e-9 {
                        seen.push(d);
                    }
                }
            }
            seen
        };
        let steady = play(&[1.0 / 60.0], false);
        let uneven = play(&[0.002, 0.07, 0.0167, 0.2, 0.004, 0.11], true);
        for (a, b) in steady.iter().zip(&uneven) {
            assert!((a.x - b.x).abs() < 2e-3 && (a.start - b.start).abs() < 2e-3 && (a.end - b.end).abs() < 2e-3, "{steady:?}\n{uneven:?}");
        }
        assert_eq!(steady.len(), marks.len());
    }

    #[test]
    fn each_part_is_put_at_its_goal_once_under_half_a_pixel_from_it() {
        let p = Params::default();
        let mut f = Follower::default();
        f.update(10.0, Reading::Locked, 1.0, 0.0, 0.0, 0.0, &p);
        f.update(10.6, Reading::Paused, 1.0, 0.0, 0.3, 0.3, &p);
        let d = run(&mut f, 0.3, 0.3, &p);
        let within = [0.01; 3];
        assert_eq!(f.settle(within, 0.6, &p), [false; 3], "still gliding: {d:?}");
        // The glide's tail under a hundredth of a word: there at once, and at rest.
        run(&mut f, 0.6, 1.5, &p);
        assert_eq!(f.settle(within, 2.1, &p), [true; 3]);
        let d = f.tick(2.1 + FRAME, FRAME as f32, &p).unwrap();
        let half = uncertainty(Reading::Paused, 1.0, 0.0) * p.spread;
        assert!(d.x == 10.6 && d.start == 10.6 - half && d.end == 10.6 + half && !f.moving(), "{d:?}");
        // Drifting, the range widens with time: its edges' goals move, the pointer's does not.
        f.update(10.6, Reading::Drifting, 0.5, 0.0, 2.2, 2.2, &p);
        run(&mut f, 2.2, 1.0, &p);
        let rest = f.settle([1.0; 3], 3.2, &p);
        assert!(rest[0] && !rest[1] && !rest[2], "{rest:?}");
        // A plan playing: its clock moves the goal, nothing is put anywhere.
        let mut g = heard_voice();
        g.update_plan(&plan(1, 10.1, 10.2, &[(2.0, 0.1)], None), Reading::Locked, 0.9, 6.0, 4.0, 4.0, &tajweed());
        play(&mut g, 4.0, 1.0, &tajweed(), true);
        assert_eq!(g.settle([1.0; 3], 5.0, &tajweed()), [false; 3]);
    }

    // Plans, the voice and the dim.

    /// Settings with tajweed timing, the hold while quiet and the dim on.
    fn tajweed() -> Params {
        Params { plan: true, voice: true, quiet_dim: 0.35, fresh: 0.55, line_stop: true, ..Default::default() }
    }

    /// Ticks from `from` for `secs` at 60 a second with the voice present (and the silence over) or not from `from` on;
    /// the pointer drawn.
    fn play(f: &mut Follower, from: f64, secs: f64, p: &Params, present: bool) -> Vec<(f64, f32)> {
        f.voice(present, !present, from);
        let (mut t, mut out) = (from, Vec::new());
        while t < from + secs - 1e-9 {
            t += FRAME;
            if let Some(d) = f.tick(t, FRAME as f32, p) {
                out.push((t, d.x));
            }
        }
        out
    }

    /// A plan on word 10 (four letters of 0.2375 words): the reader at `now`, the evidenced letter's end `ev_end`.
    fn plan(seq: u32, now: f32, ev_end: f32, knots: &[(f32, f32)], hold: Option<(f32, f32)>) -> Plan {
        let ev = (now.floor() as u32, ((now.fract() / 0.2375) as u16).min(3));
        Plan { seq, age: 0.1, ev, ev_end, now, knots: knots.to_vec(), hold }
    }

    /// A follower that has heard the voice (from clock 0).
    fn heard_voice() -> Follower {
        let mut f = Follower::default();
        f.voice(true, false, 0.0);
        f
    }

    fn last(xs: &[(f64, f32)]) -> f32 {
        xs.last().unwrap().1
    }

    fn most(xs: &[(f64, f32)]) -> f32 {
        xs.iter().fold(f32::NEG_INFINITY, |m, x| m.max(x.1))
    }

    #[test]
    fn a_held_letter_is_crept_through_and_left_on_the_next_seq() {
        let p = tajweed();
        let mut f = heard_voice();
        // The reader enters word 10's last letter, a madd held 1.2 s, and the microphone loses it to the floor.
        f.update_plan(&plan(1, 10.72, 10.95, &[(1.2, 0.23)], Some((0.0, 1.2))), Reading::Locked, 0.9, 6.0, 4.0, 4.0, &p);
        let mut held = play(&mut f, 4.0, 1.6, &p, false);
        assert!(f.dim(5.6) > 0.99, "no dim inside the hold");
        held.extend(play(&mut f, 5.6, 1.4, &p, false));
        assert!(most(&held) <= 10.95 + 1e-3, "never into the next word: {}", most(&held));
        let at = |t: f64| held.iter().find(|x| x.0 >= t).unwrap().1;
        assert!(at(5.0) > 10.85 && at(5.6) > 10.93, "crept through the hold: {} {}", at(5.0), at(5.6));
        f.update_plan(&plan(2, 11.05, 11.24, &[(0.3, 0.19)], None), Reading::Locked, 0.9, 6.0, 7.0, 7.0, &p);
        assert!(last(&play(&mut f, 7.0, 0.5, &p, true)) > 11.05);
    }

    #[test]
    fn a_plan_never_runs_past_its_evidence_the_ayah_end_or_the_stop() {
        let p = Params { fresh: 0.0, ..tajweed() };
        let far = plan(1, 10.1, 10.2, &[(2.0, 3.0)], None);
        for (end, stop, cap) in [(None, None, 10.2 + p.max_ahead), (Some(10.6), None, 10.6), (Some(10.95), Some(10.4), 10.4)] {
            let mut f = heard_voice();
            f.update_plan(&far, Reading::Locked, 0.9, 6.0, 4.0, 4.0, &p);
            f.set_ayah_end(end);
            f.set_stop(stop);
            let xs = play(&mut f, 4.0, 3.0, &p, true);
            assert!(most(&xs) <= cap + 1e-3 && last(&xs) > cap - 0.02, "{end:?} {stop:?}: {}", most(&xs));
        }
    }

    #[test]
    fn stale_evidence_waits_at_the_word_end() {
        let crossing = &[(0.3, 0.2), (1.0, 0.6)];
        for (fresh, age, crosses) in [(0.55, 0.5, false), (0.0, 0.5, true), (0.55, 0.0, true)] {
            let p = Params { fresh, ..tajweed() };
            let mut f = heard_voice();
            f.update_plan(&Plan { age, ..plan(1, 10.8, 10.95, crossing, None) }, Reading::Locked, 0.9, 6.0, 4.0, 4.0, &p);
            let x = most(&play(&mut f, 4.0, 2.0, &p, true));
            assert_eq!(x > 11.0, crosses, "fresh {fresh}, age {age}: {x}");
        }
    }

    #[test]
    fn the_clock_stops_with_the_voice_and_sound_alone_does_not_resume_a_stale_plan() {
        let p = tajweed();
        let mut f = heard_voice();
        f.update_plan(&plan(1, 10.0, 10.2, &[(2.0, 0.9)], None), Reading::Locked, 0.9, 6.0, 4.0, 4.0, &p);
        play(&mut f, 4.0, 0.5, &p, true);
        let stopped = play(&mut f, 4.5, 2.0, &p, false);
        let x = last(&stopped);
        assert!(x > 10.2 && x < 10.0 + 0.45 * 0.6, "{x}");
        let again = play(&mut f, 6.5, 1.0, &p, true);
        assert!((most(&again) - x).abs() < 1e-3, "{} from {x}", most(&again));
        f.update_plan(&plan(2, 10.5, 10.7, &[(0.4, 0.2)], None), Reading::Locked, 0.9, 6.0, 7.5, 7.5, &p);
        assert!(last(&play(&mut f, 7.5, 0.5, &p, true)) > 10.55);
    }

    #[test]
    fn a_paused_reader_is_not_stepped_back_inside_the_word_nor_from_just_past_it() {
        let p = tajweed();
        let mut f = heard_voice();
        f.update_plan(&plan(1, 10.72, 10.95, &[(1.0, 0.2)], Some((-0.2, 1.0))), Reading::Locked, 0.9, 6.0, 4.0, 4.0, &p);
        let before = last(&play(&mut f, 4.0, 0.7, &p, true));
        f.update(10.6, Reading::Paused, 0.9, 6.0, 4.7, 4.7, &p);
        let paused = play(&mut f, 4.7, 2.0, &p, false);
        assert!(paused.iter().all(|x| x.1 >= before - 1e-4), "nothing back from {before}");
        for (reach, rests) in [(0.25, true), (0.55, false)] {
            let mut f = heard_voice();
            f.update_plan(&Plan { age: 0.0, ..plan(1, 10.9, 10.95, &[(0.5, reach)], None) }, Reading::Locked, 0.9, 6.0, 4.0, 4.0, &p);
            let before = last(&play(&mut f, 4.0, 1.0, &p, true));
            assert!(before > 11.0, "{before}");
            f.update(10.83, Reading::Paused, 0.9, 6.0, 5.0, 5.0, &p);
            let x = last(&play(&mut f, 5.0, 2.0, &p, false));
            assert!(if rests { (x - before).abs() < 1e-3 } else { (x - 10.95).abs() < 0.02 }, "reach {reach}: {before} then {x}");
        }
    }

    #[test]
    fn a_tentative_excursion_waits_a_block_and_its_return_is_followed_at_once() {
        let p = tajweed();
        let locked = |f: &mut Follower, seq, now: f32, t: f64| {
            let pl = Plan { ev: (20, 1), ev_end: 20.475, ..plan(seq, now, 0.0, &[(0.3, 0.1)], None) };
            f.update_plan(&pl, Reading::Locked, 0.9, 6.0, t as f32, t, &p);
        };
        let mut f = heard_voice();
        locked(&mut f, 1, 20.3, 4.0);
        play(&mut f, 4.0, 0.32, &p, true);
        f.update(23.5, Reading::Tentative, 0.5, 6.0, 4.32, 4.32, &p);
        assert_eq!(f.wake(4.32, &p).map(|w| (w * 100.0).round()), Some(32.0), "a frame when its block is over");
        let held = play(&mut f, 4.32, 0.3, &p, true);
        locked(&mut f, 2, 20.6, 4.62);
        let back = play(&mut f, 4.62, 1.0, &p, true);
        assert!(most(&held).max(most(&back)) < 21.0, "{} {}", most(&held), most(&back));
        let mut f = heard_voice();
        locked(&mut f, 1, 20.3, 4.0);
        play(&mut f, 4.0, 0.32, &p, true);
        f.update(23.5, Reading::Tentative, 0.5, 6.0, 4.32, 4.32, &p);
        let out = play(&mut f, 4.32, 1.0, &p, true);
        assert!(out.iter().find(|x| x.0 >= 4.6).unwrap().1 < 21.0 && last(&out) > 22.5, "a block late: {}", last(&out));
        f.update(20.7, Reading::Locked, 0.9, 6.0, 5.32, 5.32, &p);
        assert!(last(&play(&mut f, 5.32, 0.8, &p, true)) < 21.5, "back without the wait");
    }

    #[test]
    fn it_dims_in_silence_and_comes_back_and_not_inside_a_hold() {
        let p = Params { quiet_delay: 1.0, ..tajweed() };
        let mut f = heard_voice();
        f.update_plan(&plan(1, 10.1, 10.2, &[(0.3, 0.1)], None), Reading::Locked, 0.9, 6.0, 4.0, 4.0, &p);
        play(&mut f, 4.0, 1.0, &p, true);
        assert!(f.dim(5.0) > 0.999);
        play(&mut f, 5.0, 0.9, &p, false);
        assert!(f.dim(5.9) > 0.99 && f.wake(5.9, &p).is_some_and(|w| (w - 0.1).abs() < 0.02), "the dim wakes the app when it starts");
        play(&mut f, 5.9, 1.6, &p, false);
        assert!((f.dim(7.5) - p.quiet_dim).abs() < 0.05, "{}", f.dim(7.5));
        play(&mut f, 7.5, 0.4, &p, true);
        assert!(f.dim(7.9) > 0.95, "back: {}", f.dim(7.9));
        let mut f = heard_voice();
        f.update_plan(&plan(1, 10.72, 10.95, &[(3.0, 0.23)], Some((0.0, 3.0))), Reading::Locked, 0.9, 6.0, 4.0, 4.0, &p);
        play(&mut f, 4.0, 4.0, &p, false);
        assert!(f.dim(8.0) > 0.999, "{}", f.dim(8.0));
        let never = Params { quiet_dim: 1.0, ..p };
        let mut f = heard_voice();
        f.update_plan(&plan(1, 10.1, 10.2, &[(0.3, 0.1)], None), Reading::Locked, 0.9, 6.0, 4.0, 4.0, &never);
        play(&mut f, 4.0, 5.0, &never, false);
        assert!(f.dim(9.0) > 0.999);
    }

    #[test]
    fn without_plans_the_run_on_counts_only_while_the_voice_is_present() {
        let p = tajweed();
        let read = |voiced: bool| {
            let mut f = heard_voice();
            for k in 0..6 {
                let t = 4.0 + f64::from(k) * 0.32;
                f.update(10.0 + k as f32 * 0.4, Reading::Locked, 1.0, 7.0, t as f32, t, &p);
                if k < 5 {
                    play(&mut f, t, 0.32, &p, true);
                }
            }
            last(&play(&mut f, 5.6, 2.0, &p, voiced))
        };
        let (on, stopped) = (read(true), read(false));
        assert!((on - (12.0 + p.max_ahead)).abs() < 0.01 && stopped < 12.0 + 0.2 * p.max_ahead, "{on} {stopped}");
        let mut g = heard_voice();
        g.update_plan(&plan(1, 10.1, 10.2, &[], None), Reading::Locked, 0.9, 7.0, 4.0, 4.0, &p);
        g.update(10.5, Reading::Drifting, 0.5, 7.0, 4.32, 4.32, &p);
        let x = most(&play(&mut g, 4.32, 1.5, &p, true));
        assert!(x <= 10.5 + 1e-3 && x > 10.45, "Drifting has no run-on of its own once plans have come: {x}");
    }

    /// Presence and the dim off: the plan's arithmetic alone.
    fn plain() -> Params {
        Params { voice: false, quiet_dim: 1.0, ..tajweed() }
    }

    /// Ticks from `from` for `secs` with no news of the voice; the pointer drawn last.
    fn quietly(f: &mut Follower, from: f64, secs: f64, p: &Params) -> f32 {
        run(f, from, secs, p).x
    }

    #[test]
    fn a_stale_plan_already_in_the_next_word_goes_on() {
        // Its evidence aged, a plan is held at the evidenced word's end only while the plan itself (not the glide
        // trailing it) is still inside that word: here `now` is already 0.07 into the next.
        let p = plain();
        let mut f = Follower::default();
        f.update_plan(&Plan { age: 0.0, ..plan(1, 10.5, 10.7125, &[], None) }, Reading::Locked, 0.9, 6.0, 4.0, 4.0, &p);
        quietly(&mut f, 4.0, 2.0, &p);
        let on = Plan { seq: 2, age: 0.4, ev: (10, 3), ev_end: 10.95, now: 11.07, knots: vec![(0.5, 0.2)], hold: None };
        f.update_plan(&on, Reading::Locked, 0.9, 6.0, 6.0, 6.0, &p);
        let most = (1..=90).map(|k| f.tick(6.0 + f64::from(k) * FRAME, FRAME as f32, &p).unwrap().x).fold(f32::MIN, f32::max);
        assert!(most > 11.2, "{most}");
    }

    #[test]
    fn a_goal_near_the_end_of_the_quran_is_reached() {
        // From word 65,536 on an f32 is spaced 0.0078: a glide's step under half that would be lost, 0.05 word short.
        let p = plain();
        for w in [10u32, 30_000, 72_500, 73_000, 77_400] {
            let wf = w as f32;
            let mut f = Follower::default();
            f.update_plan(&Plan { seq: 1, age: 0.0, ev: (w, 0), ev_end: wf + 0.2375, now: wf + 0.1, knots: vec![(0.3, 0.3)], hold: None }, Reading::Locked, 0.9, 6.0, 4.0, 4.0, &p);
            let x = quietly(&mut f, 4.0, 3.0, &p);
            assert!(((wf + 0.1) + 0.3 - x).abs() < 0.005, "word {w}: {x}");
        }
    }

    #[test]
    fn the_line_stop_holds_against_the_plans_lead() {
        // The evidence on a line's last word (102), the plan already past the line's end by the display lead: the pointer
        // stays on the line, a first message (a respawn) too.
        let p = tajweed();
        for now in [102.8f32, 103.1] {
            let mut f = heard_voice();
            let pl = Plan { seq: 1, age: 0.1, ev: (102, 3), ev_end: 102.95, now, knots: vec![(0.3, 0.1), (0.8, 0.3)], hold: None };
            f.update_plan(&pl, Reading::Locked, 0.9, 6.0, 4.0, 4.0, &p);
            f.set_ayah_end(Some(110.95));
            f.set_stop(Some(102.95));
            let xs = play(&mut f, 4.0, 1.0, &p, true);
            assert!(most(&xs) <= 102.95 + 1e-3 && last(&xs) > 102.9, "now {now}: {}", most(&xs));
        }
    }

    #[test]
    fn tajweed_timing_turned_off_moves_as_if_no_plan_had_come() {
        let on = plain();
        let off = Params::default();
        assert!(!off.plan);
        let drawn = |had_plan: bool| {
            let mut f = Follower::default();
            for k in 0..6 {
                let t = f64::from(k) * 0.32;
                f.update(10.0 + k as f32 * 0.4, Reading::Locked, 0.9, 7.6, t as f32, t, &off);
                quietly(&mut f, t, 0.32, &off);
            }
            if had_plan {
                f.update_plan(&Plan { seq: 1, age: 0.1, ev: (12, 0), ev_end: 12.2, now: 12.1, knots: vec![(0.3, 0.1)], hold: None }, Reading::Locked, 0.9, 7.6, 1.92, 1.92, &on);
            } else {
                f.update(12.1, Reading::Locked, 0.9, 7.6, 1.92, 1.92, &off);
            }
            quietly(&mut f, 1.92, 0.32, &off);
            // Off from here: Drifting runs on again, a Tentative jump is shown at once.
            f.update(12.3, Reading::Drifting, 0.5, 7.6, 2.24, 2.24, &off);
            let drift = quietly(&mut f, 2.24, 1.0, &off);
            f.update(12.6, Reading::Locked, 0.9, 7.6, 3.24, 3.24, &off);
            quietly(&mut f, 3.24, 0.1, &off);
            f.update(15.8, Reading::Tentative, 0.5, 7.6, 3.34, 3.34, &off);
            (drift, quietly(&mut f, 3.34, 0.2, &off))
        };
        let (never, after) = (drawn(false), drawn(true));
        assert!((never.0 - after.0).abs() < 0.02 && (never.1 - after.1).abs() < 0.05 && after.0 > 12.6 && after.1 > 13.5, "{never:?} {after:?}");
    }

    #[test]
    fn it_dims_by_the_level_whether_or_not_the_run_on_is_held() {
        for voice in [false, true] {
            let p = Params { voice, quiet_dim: 0.35, quiet_delay: 1.0, ..Params::default() };
            let mut f = heard_voice();
            f.update(10.2, Reading::Locked, 0.9, 6.0, 4.0, 4.0, &p);
            play(&mut f, 4.0, 1.0, &p, true);
            play(&mut f, 5.0, 2.4, &p, false);
            assert!(f.dim(7.4) < 0.4, "voice held {voice}: {}", f.dim(7.4));
        }
    }

    #[test]
    fn the_same_seq_after_a_long_pause_is_no_news() {
        let p = Params { quiet_delay: 1.0, ..tajweed() };
        let mut f = heard_voice();
        // The reader stops inside word 10, heard on its second letter; the plan would run on into word 11.
        let pl = Plan { seq: 7, age: 0.1, ev: (10, 1), ev_end: 10.475, now: 10.4, knots: vec![(0.4, 0.3), (1.0, 0.75)], hold: None };
        f.update_plan(&pl, Reading::Locked, 0.9, 6.0, 4.0, 4.0, &p);
        play(&mut f, 4.0, 0.3, &p, true);
        f.update(10.4, Reading::Paused, 0.9, 6.0, 4.3, 4.3, &p);
        let before = last(&play(&mut f, 4.3, 6.0, &p, false));
        assert!(f.dim(10.3) < 0.4, "dimmed in the pause: {}", f.dim(10.3));
        // Locked again 6 s on with the same seq, its `now` run on by its age into the next word, the room still quiet.
        let again = Plan { age: 6.4, now: 11.05, ..pl.clone() };
        f.update_plan(&again, Reading::Locked, 0.9, 6.0, 10.3, 10.3, &p);
        let xs = play(&mut f, 10.3, 1.5, &p, false);
        assert!(most(&xs) <= before + 1e-3, "{before} then {}", most(&xs));
        assert!(f.dim(11.8) < 0.4, "no undim: {}", f.dim(11.8));
        // The same with a message between (the plan set aside by a Tentative one): still no news.
        let mut g = heard_voice();
        g.update_plan(&pl, Reading::Locked, 0.9, 6.0, 4.0, 4.0, &p);
        play(&mut g, 4.0, 0.3, &p, true);
        g.update(10.45, Reading::Tentative, 0.6, 6.0, 4.3, 4.3, &p);
        let before = last(&play(&mut g, 4.3, 0.32, &p, true));
        g.update_plan(&again, Reading::Locked, 0.9, 6.0, 4.62, 4.62, &p);
        assert!(most(&play(&mut g, 4.62, 1.0, &p, true)) <= before.max(pl.ev_end) + 1e-3);
    }

    #[test]
    fn a_short_return_of_the_voice_does_not_restart_the_dim() {
        // The link's presence: silent from 5 s, back at 5.9 s for less than a blip (present, the silence going on).
        let p = Params { quiet_delay: 1.0, ..tajweed() };
        let mut f = heard_voice();
        f.update_plan(&plan(1, 10.1, 10.95, &[(2.0, 0.8)], None), Reading::Locked, 0.9, 6.0, 4.0, 4.0, &p);
        play(&mut f, 4.0, 1.0, &p, true);
        let quiet = last(&play(&mut f, 5.0, 0.9, &p, false));
        f.voice(true, true, 5.9);
        run(&mut f, 5.9, 0.2, &p);
        f.voice(false, true, 6.1);
        let x = run(&mut f, 6.1, 0.5, &p).x;
        assert!((x - quiet).abs() < 0.01 && f.dim(6.6) < 0.6, "{quiet} {x} {}", f.dim(6.6));
    }

    #[test]
    fn news_in_a_long_silence_waits_for_the_voice() {
        // The listener steps on in a pause of 2 s (hearing the room): nothing moves until the reader is heard again.
        let p = tajweed();
        let mut f = heard_voice();
        f.update_plan(&plan(1, 10.6, 10.7125, &[(0.3, 0.1)], None), Reading::Locked, 0.9, 6.0, 4.0, 4.0, &p);
        play(&mut f, 4.0, 0.5, &p, true);
        let rest = last(&play(&mut f, 4.5, 1.8, &p, false));
        f.update_plan(&Plan { age: 0.05, ..plan(2, 11.3, 11.475, &[(0.3, 0.2)], None) }, Reading::Locked, 0.9, 6.0, 6.3, 6.3, &p);
        assert!(most(&play(&mut f, 6.3, 0.6, &p, false)) <= rest + 1e-3);
        assert!(last(&play(&mut f, 6.9, 0.6, &p, true)) > 11.2, "the voice back");
        // A far jump heard in a long silence lands and stays, not pulled back to where earlier news found the pointer;
        // with the voice back it goes on, the hold let go without a jump.
        play(&mut f, 7.5, 1.5, &p, false);
        f.update(30.5, Reading::Tentative, 0.5, 6.0, 9.0, 9.0, &p);
        let far = play(&mut f, 9.0, 2.0, &p, false);
        assert!(far.iter().all(|x| (x.1 - 30.5).abs() < 1e-3), "{:?}", far.last());
        let back = play(&mut f, 11.0, 1.0, &p, true);
        assert!(back.windows(2).all(|w| (w[1].1 - w[0].1).abs() < 0.1), "{back:?}");
        // Shorter silences keep following the listener: the evidence of the last sounds comes late.
        let mut g = heard_voice();
        g.update_plan(&plan(1, 10.6, 10.7125, &[(0.3, 0.1)], None), Reading::Locked, 0.9, 6.0, 4.0, 4.0, &p);
        play(&mut g, 4.0, 0.5, &p, true);
        play(&mut g, 4.5, 0.5, &p, false);
        g.update_plan(&Plan { age: 0.05, ..plan(2, 11.3, 11.475, &[(0.3, 0.2)], None) }, Reading::Locked, 0.9, 6.0, 5.0, 5.0, &p);
        assert!(last(&play(&mut g, 5.0, 0.6, &p, false)) > 11.2);
    }
}
