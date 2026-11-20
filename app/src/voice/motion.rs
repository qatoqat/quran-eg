//! How the voice pointer and its accuracy range move between the listener's updates: the follower of
//! quran-voice's `qv-motion` (the quran.com extension moves its pointer the same way). Positions are in
//! words (word index + fraction through the word). Updates arrive every ~320 ms and are bursty (a word
//! held, then several at once), so the pointer runs on at the reader's measured pace, never more than
//! `max_ahead` past the evidence, and eases toward each new estimate along its curve; the range's two
//! edges ease to either side of it, each by its own curve. When the evidence falls behind the pointer,
//! the pointer waits (`wait`) before easing back, while the range's start reaches back to the reader;
//! going back more than `fade_back` words, or forward more than `far_jump`, is a jump: the pointer
//! fades over instead.
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
}

impl Params {
    pub fn of(v: &Voice) -> Params {
        let m = &v.motion;
        let ease = |g: VoiceGlide| Ease { curve: g.curve, secs: f32::from(g.ms) / 1000.0 };
        Params {
            pointer: ease(m.pointer),
            start: ease(m.range_start),
            end: ease(m.range_end),
            spread: f32::from(v.range.spread) / 10.0,
            fade_back: f32::from(m.fade_back) / 10.0,
            far_jump: f32::from(m.far_jump),
            max_ahead: f32::from(m.max_ahead) / 10.0,
            wait: f32::from(m.wait_ms) / 1000.0,
            ayah_fade: m.fade_at_ayah_end.then(|| f32::from(m.ayah_fade_delay_ms) / 1000.0),
        }
    }
}

impl Default for Params {
    fn default() -> Self {
        Params::of(&Voice::default())
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

const UNITS_PER_WORD: f32 = 6.15;

#[derive(Default)]
pub struct Follower {
    target: Option<f32>,
    reading: Option<Reading>,
    conf: f32,
    /// Clock time (s) of the last update.
    recv: f64,
    pointer: Glider,
    start: Glider,
    end: Glider,
    /// Where the pointer, the start and the end were going on the last tick, and whether any of them
    /// was still on its way (or its goal moving).
    goals: [f32; 3],
    moving: bool,
    /// Locked evidence of the last seconds as (position, audio time), for the reader's pace.
    seen: VecDeque<(f32, f32)>,
    pace: f32,
    behind_since: Option<f64>,
    /// How far it may run ahead without evidence: the end of the ayah being read (readers stop there).
    ayah_end: Option<f32>,
    /// Since when it has been parked at the end of the ayah just read (`Params::ayah_fade`): since the
    /// reader last sounded its last word.
    parked: Option<f64>,
}

impl Follower {
    /// The furthest position the pointer may run ahead to before the reader is heard there: the end of
    /// the ayah being read (just inside its last word). `None` = no limit.
    pub fn set_ayah_end(&mut self, end: Option<f32>) {
        self.ayah_end = end;
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update(&mut self, c: f32, reading: Reading, conf: f32, speed_units: f32, t_audio: f32, now: f64, p: &Params) -> Jump {
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
        self.target = Some(c);
        self.reading = Some(reading);
        self.conf = conf;
        self.recv = now;
        jump
    }

    /// The tracker lost the place (searching, or the word is not on this page).
    pub fn hide(&mut self) {
        self.target = None;
        self.parked = None;
    }

    /// Parked at the end of the ayah just read for at least the delay: the view fades the pointer out
    /// until the next update elsewhere (`Jump::Respawn` there).
    pub fn faded(&self, now: f64, p: &Params) -> bool {
        matches!((self.parked, p.ayah_fade), (Some(since), Some(delay)) if now - since >= f64::from(delay))
    }

    /// The pointer or an edge of the range moved on the last tick, or is still on its way.
    pub fn moving(&self) -> bool {
        self.moving
    }

    /// Seconds until something changes with no news from the listener: the end of the wait for a
    /// reader who is behind, or of the pause before a parked pointer fades out.
    pub fn wake(&self, now: f64, p: &Params) -> Option<f32> {
        // In f64 as `faded` and `tick` count: a frame a hair early must still ask for the next one.
        let left = |since: f64, secs: f32| {
            let left = since + f64::from(secs) - now;
            (left > 0.0).then_some(left as f32)
        };
        let wait = self.behind_since.filter(|_| self.reading != Some(Reading::Paused)).and_then(|since| left(since, p.wait));
        let fade = self.parked.zip(p.ayah_fade).and_then(|(since, delay)| left(since, delay));
        wait.into_iter().chain(fade).reduce(f32::min)
    }

    fn goal(&self, target: f32, now: f64, p: &Params) -> f32 {
        let dt = ((now - self.recv) as f32).clamp(0.0, 1.0);
        let v = match self.reading {
            Some(Reading::Paused) => 0.0,
            Some(Reading::Tentative) => self.pace * 0.5,
            _ => self.pace,
        };
        target + (v * dt).min(p.max_ahead)
    }

    /// Advances the pointer and the range's edges to clock time `now` (s).
    pub fn tick(&mut self, now: f64, dt: f32, p: &Params) -> Option<Drawn> {
        let target = self.target?;
        let reading = self.reading?;
        // Where the reader is heard, with the run-ahead.
        let mut est = self.goal(target, now, p);
        if let Some(end) = self.ayah_end {
            est = est.min(end.max(target));
        }
        let mut goal = est;
        if est < self.pointer.x {
            // Ahead of the reader: wait for them (the evidence comes in bursts), and only then ease
            // back; a reader who has stopped is not coming, so then at once.
            let since = *self.behind_since.get_or_insert(now);
            if now - since < f64::from(p.wait) && reading != Reading::Paused {
                goal = self.pointer.x;
            }
        } else {
            self.behind_since = None;
        }
        // The start reaches back to where the reader is heard even while the pointer waits; the end
        // stays ahead of the pointer.
        let half = uncertainty(reading, self.conf, (now - self.recv) as f32) * p.spread;
        let goals = [goal, est - half, goal + half];
        let x = self.pointer.step(goals[0], now, dt, p.pointer);
        let start = self.start.step(goals[1], now, dt, p.start).min(x);
        let end = self.end.step(goals[2], now, dt, p.end).max(x);
        let parts = [&self.pointer, &self.start, &self.end];
        self.moving = goals != self.goals || parts.iter().zip(goals).any(|(part, goal)| part.moving(goal));
        self.goals = goals;
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
        // A time-based curve sits on its goal once its glide is over; the goal still runs on.
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
}
