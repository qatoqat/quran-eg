//! How the voice pointer moves between the listener's updates: the follower of quran-voice's
//! `qv-motion` (the quran.com extension moves its pointer the same way). Positions are in words
//! (word index + fraction through the word). Updates arrive every ~320 ms and are bursty (a word held,
//! then several at once), so the pointer runs on at the reader's measured pace, never more than
//! `max_ahead` past the evidence, and eases toward each new estimate along the chosen curve. When the
//! evidence falls behind it, the pointer waits (`wait`) before easing back; going back more than
//! `fade_back` words, or forward more than `far_jump`, is a jump: the pointer fades over instead.
use std::collections::VecDeque;

use quran_core::settings::{VoiceCurve as Curve, VoiceMotion};

/// Progress along a glide (0..=1) for the time-based curves.
fn ease(curve: Curve, u: f32) -> f32 {
    let u = u.clamp(0.0, 1.0);
    match curve {
        Curve::Linear => u,
        Curve::EaseOut => 1.0 - (1.0 - u).powi(3),
        Curve::EaseInOut => {
            if u < 0.5 {
                4.0 * u * u * u
            } else {
                1.0 - (-2.0 * u + 2.0).powi(3) / 2.0
            }
        }
        Curve::Smooth | Curve::Spring => 1.0 - (-5.0 * u).exp(),
    }
}

pub const CURVES: [(Curve, &str); 5] =
    [(Curve::Smooth, "Smooth"), (Curve::Spring, "Spring"), (Curve::Linear, "Linear"), (Curve::EaseOut, "Ease out"), (Curve::EaseInOut, "Ease in-out")];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reading {
    Locked,
    Tentative,
    Drifting,
    Paused,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Params {
    pub curve: Curve,
    /// Seconds a glide takes (time constant for Smooth and Spring).
    pub glide: f32,
    pub fade_back: f32,
    pub far_jump: f32,
    pub max_ahead: f32,
    /// Seconds to wait when the evidence is behind the pointer before easing back.
    pub wait: f32,
}

impl Params {
    pub fn of(m: &VoiceMotion) -> Params {
        Params {
            curve: m.curve,
            glide: f32::from(m.glide_ms) / 1000.0,
            fade_back: f32::from(m.fade_back) / 10.0,
            far_jump: f32::from(m.far_jump),
            max_ahead: f32::from(m.max_ahead) / 10.0,
            wait: f32::from(m.wait_ms) / 1000.0,
        }
    }
}

impl Default for Params {
    fn default() -> Self {
        Params::of(&VoiceMotion::default())
    }
}

/// What an update asks the view to do with the drawn pointer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Jump {
    Glide,
    /// Fade the pointer out where it is and in at the new place.
    Respawn,
}

const UNITS_PER_WORD: f32 = 6.15;

#[derive(Default)]
pub struct Follower {
    target: Option<f32>,
    reading: Option<Reading>,
    conf: f32,
    /// Clock time (s) of the last update and the drawn position when it came.
    recv: f64,
    from: f32,
    x: f32,
    v: f32,
    /// Locked evidence of the last seconds as (position, audio time), for the reader's pace.
    seen: VecDeque<(f32, f32)>,
    pace: f32,
    sigma: f32,
    behind_since: Option<f64>,
    /// How far it may run ahead without evidence: the end of the ayah being read (readers stop there).
    end: Option<f32>,
}

impl Follower {
    /// The furthest position the pointer may run ahead to before the reader is heard there: the end of
    /// the ayah being read (just inside its last word). `None` = no limit.
    pub fn set_end(&mut self, end: Option<f32>) {
        self.end = end;
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update(&mut self, c: f32, reading: Reading, conf: f32, speed_units: f32, t_audio: f32, now: f64, p: &Params) -> Jump {
        let last = self.seen.back().map(|s| s.0);
        let went_back = self.target.is_some() && last.is_some_and(|l| c < l - p.fade_back) && c < self.x - p.fade_back;
        let jump = if self.target.is_none() || went_back || c > self.x + p.far_jump { Jump::Respawn } else { Jump::Glide };
        if jump == Jump::Respawn {
            self.x = c;
            self.v = 0.0;
            self.seen.clear();
            self.behind_since = None;
            self.sigma = uncertainty(reading, conf, 0.0);
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
        self.from = self.x;
        jump
    }

    /// The tracker lost the place (searching, or the word is not on this page).
    pub fn hide(&mut self) {
        self.target = None;
    }

    pub fn pace(&self) -> f32 {
        self.pace
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

    /// Advances the drawn position to clock time `now` (s); returns it and the uncertainty in words.
    pub fn tick(&mut self, now: f64, dt: f32, p: &Params) -> Option<(f32, f32)> {
        let target = self.target?;
        let reading = self.reading?;
        let mut goal = self.goal(target, now, p);
        if let Some(end) = self.end {
            goal = goal.min(end.max(target));
        }
        if goal < self.x {
            // Ahead of the reader: wait for them (the evidence comes in bursts), and only then ease
            // back; a reader who has stopped is not coming, so then at once.
            let since = *self.behind_since.get_or_insert(now);
            if now - since < p.wait as f64 && reading != Reading::Paused {
                goal = self.x;
            }
        } else {
            self.behind_since = None;
        }
        let glide = p.glide.max(0.02);
        match p.curve {
            Curve::Smooth => {
                let tau = if goal - self.x > 2.0 { glide * 1.5 } else { glide };
                self.x += (goal - self.x) * (1.0 - (-dt / tau).exp());
            }
            Curve::Spring => {
                let w = 4.0 / glide;
                self.v += (w * w * (goal - self.x) - 2.0 * w * self.v) * dt;
                if goal >= self.x && self.v < 0.0 {
                    self.v = 0.0;
                }
                self.x += self.v * dt;
            }
            curve => {
                if goal == self.x {
                    self.from = self.x;
                } else {
                    let u = (now - self.recv) as f32 / glide;
                    self.x = self.from + (goal - self.from) * ease(curve, u);
                }
            }
        }
        let since = (now - self.recv) as f32;
        self.sigma += (uncertainty(reading, self.conf, since) - self.sigma) * (1.0 - (-dt / 0.3).exp());
        Some((self.x, self.sigma))
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
    use super::*;

    fn run(f: &mut Follower, from: f64, secs: f64, p: &Params) -> f32 {
        let mut t = from;
        let mut x = 0.0;
        while t < from + secs {
            t += 1.0 / 60.0;
            x = f.tick(t, 1.0 / 60.0, p).unwrap().0;
        }
        x
    }

    #[test]
    fn curves_start_at_zero_end_at_one_and_never_go_back() {
        for (c, _) in CURVES {
            assert!(ease(c, 0.0).abs() < 1e-6, "{c:?}");
            assert!((ease(c, 1.0) - 1.0).abs() < 0.01, "{c:?}");
            let mut prev = 0.0;
            for i in 1..=100 {
                let e = ease(c, i as f32 / 100.0);
                assert!(e >= prev - 1e-6, "{c:?} not monotonic");
                prev = e;
            }
        }
    }

    #[test]
    fn every_curve_glides_to_a_held_word() {
        for (curve, _) in CURVES {
            let p = Params { curve, ..Default::default() };
            let mut f = Follower::default();
            assert_eq!(f.update(3.5, Reading::Locked, 1.0, 0.0, 0.0, 0.0, &p), Jump::Respawn);
            f.update(5.5, Reading::Paused, 1.0, 0.0, 0.3, 0.3, &p);
            let x = run(&mut f, 0.3, 3.0, &p);
            assert!((x - 5.5).abs() < 0.05, "{curve:?} ended at {x}");
        }
    }

    #[test]
    fn runs_on_at_pace_but_not_past_max_ahead() {
        let p = Params::default();
        let mut f = Follower::default();
        for k in 0..10 {
            f.update(k as f32 * 0.5, Reading::Locked, 1.0, 0.0, k as f32 * 0.32, k as f64 * 0.32, &p);
        }
        let x = run(&mut f, 9.0 * 0.32, 3.0, &p);
        assert!(x <= 4.5 + p.max_ahead + 1e-3 && x > 4.5, "x {x}");
    }

    #[test]
    fn a_small_step_back_waits_then_eases_back() {
        let p = Params { wait: 1.0, ..Default::default() };
        let mut f = Follower::default();
        f.update(10.0, Reading::Locked, 1.0, 0.0, 0.0, 0.0, &p);
        f.update(10.0, Reading::Paused, 1.0, 0.0, 0.3, 0.3, &p);
        run(&mut f, 0.3, 1.0, &p);
        assert_eq!(f.update(9.4, Reading::Locked, 1.0, 0.0, 1.3, 1.3, &p), Jump::Glide);
        let held = run(&mut f, 1.3, 0.8, &p);
        assert!((held - 10.0).abs() < 1e-3, "should wait, at {held}");
        let back = run(&mut f, 2.1, 2.0, &p);
        assert!((back - 9.4).abs() < 0.2, "should ease back, at {back}");
    }

    #[test]
    fn a_paused_reader_is_not_waited_for_and_the_ayah_end_holds() {
        let p = Params::default();
        let mut f = Follower::default();
        f.update(10.0, Reading::Locked, 1.0, 0.0, 0.0, 0.0, &p);
        run(&mut f, 0.0, 1.0, &p);
        f.update(9.6, Reading::Paused, 1.0, 0.0, 1.0, 1.0, &p);
        assert!((run(&mut f, 1.0, 1.0, &p) - 9.6).abs() < 0.05);
        let mut g = Follower::default();
        for k in 0..8 {
            g.update(5.0 + k as f32, Reading::Locked, 1.0, 0.0, k as f32, f64::from(k), &p);
            g.set_end(Some(12.95));
        }
        assert!(run(&mut g, 7.0, 3.0, &p) <= 12.95 + 1e-3);
    }

    #[test]
    fn going_back_far_or_jumping_far_ahead_respawns() {
        let p = Params::default();
        let mut f = Follower::default();
        f.update(20.0, Reading::Locked, 1.0, 0.0, 0.0, 0.0, &p);
        run(&mut f, 0.0, 0.5, &p);
        assert_eq!(f.update(17.0, Reading::Locked, 1.0, 0.0, 0.5, 0.5, &p), Jump::Respawn);
        assert_eq!(f.update(17.0 + p.far_jump + 1.0, Reading::Locked, 1.0, 0.0, 0.8, 0.8, &p), Jump::Respawn);
        assert_eq!(f.update(31.0, Reading::Locked, 1.0, 0.0, 1.1, 1.1, &p), Jump::Glide);
    }

    #[test]
    fn uncertainty_grows_while_drifting() {
        assert!(uncertainty(Reading::Locked, 1.0, 0.0) < uncertainty(Reading::Tentative, 1.0, 0.0));
        assert!(uncertainty(Reading::Drifting, 1.0, 2.0) > uncertainty(Reading::Drifting, 1.0, 0.5));
        assert!(uncertainty(Reading::Drifting, 1.0, 99.0) <= 4.0);
    }
}
