//! One eased value of the voice pointer (the pointer, or an edge of its range): it chases a goal by
//! its own curve and time, so parts with different settings move apart and settle together.

use quran_core::settings::VoiceCurve as Curve;

pub const CURVES: [(Curve, &str); 3] = [(Curve::Smooth, "Smooth"), (Curve::Spring, "Spring"), (Curve::Linear, "Linear")];

/// How a part eases: its curve and the seconds a glide takes (the time constant for Smooth and Spring).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ease {
    pub curve: Curve,
    pub secs: f32,
}

#[derive(Default, Clone, Copy, Debug)]
pub struct Glider {
    pub x: f32,
    v: f32,
    /// Where the current glide of the time-based curve (Linear) started, and when.
    from: f32,
    t0: f64,
    /// The place to the last bit, which `step_exact` and `advance` keep.
    exact: f64,
}

impl Glider {
    /// Put it at `x`, at rest.
    pub fn snap(&mut self, x: f32, now: f64) {
        *self = Glider { x, v: 0.0, from: x, t0: now, exact: f64::from(x) };
    }

    /// Where it is to the last bit: what `step_exact` and `advance` left, or `x` once it was moved otherwise.
    pub fn at(&self) -> f64 {
        if self.exact as f32 == self.x { self.exact } else { f64::from(self.x) }
    }

    /// Moves on by `by`, no further than `goal`, to the last bit.
    pub fn advance(&mut self, by: f32, goal: f32) {
        let x = (self.at() + f64::from(by)).min(f64::from(goal));
        (self.exact, self.x) = (x, x as f32);
    }

    /// A new place was heard: Linear glides on from here, now.
    pub fn retarget(&mut self, now: f64) {
        self.from = self.x;
        self.t0 = now;
    }

    /// Moves toward `goal` to clock time `now` (s), `dt` after the last step; returns where it is.
    pub fn step(&mut self, goal: f32, now: f64, dt: f32, e: Ease) -> f32 {
        let secs = e.secs.max(0.02);
        match e.curve {
            Curve::Smooth => {
                let tau = if goal - self.x > 2.0 { secs * 1.5 } else { secs };
                self.x += (goal - self.x) * (1.0 - (-dt / tau).exp());
            }
            Curve::Spring => {
                // Critically damped, stepped exactly: stable at any step (an Euler step blows up at
                // the shortest glide). Never sliding back while the goal is ahead (from there it does
                // not within a step either).
                let w = 4.0 / secs;
                if goal >= self.x && self.v < 0.0 {
                    self.v = 0.0;
                }
                let e = self.x - goal;
                let k = (self.v + w * e) * dt;
                let d = (-w * dt).exp();
                self.x = goal + (e + k) * d;
                self.v = (self.v - w * k) * d;
            }
            Curve::Linear => {
                if goal == self.x {
                    // At rest: the next glide starts here, then (not at the last update, long ago).
                    self.retarget(now);
                } else {
                    let u = ((now - self.t0) as f32 / secs).clamp(0.0, 1.0);
                    self.x = self.from + (goal - self.from) * u;
                }
            }
        }
        self.x
    }

    /// `step` to the last bit: from word 65,536 on an f32 is spaced 0.0078, and a glide's step under half that is lost,
    /// so at the Quran's end the pointer stalled 0.03-0.07 word short of each goal.
    pub fn step_exact(&mut self, goal: f32, now: f64, dt: f32, e: Ease) -> f32 {
        let (mut x, g, secs) = (self.at(), f64::from(goal), e.secs.max(0.02));
        match e.curve {
            Curve::Smooth => {
                let tau = if g - x > 2.0 { secs * 1.5 } else { secs };
                x += (g - x) * f64::from(1.0 - (-dt / tau).exp());
            }
            Curve::Spring => {
                let (w, dt) = (f64::from(4.0 / secs), f64::from(dt));
                if g >= x && self.v < 0.0 {
                    self.v = 0.0;
                }
                let e = x - g;
                let k = (f64::from(self.v) + w * e) * dt;
                let d = (-w * dt).exp();
                x = g + (e + k) * d;
                self.v = ((f64::from(self.v) - w * k) * d) as f32;
            }
            Curve::Linear => {
                if g == x {
                    self.retarget(now);
                } else {
                    let u = ((now - self.t0) as f32 / secs).clamp(0.0, 1.0);
                    x = f64::from(self.from) + (g - f64::from(self.from)) * f64::from(u);
                }
            }
        }
        (self.exact, self.x) = (x, x as f32);
        self.x
    }

    /// Still on its way to `goal`.
    #[cfg(test)]
    pub fn moving(&self, goal: f32) -> bool {
        (goal - self.x).abs() > 0.002 || self.v.abs() > 0.002
    }

    /// At most how far it has still to go to `goal` by its curve: a spring also carries on by its speed (over a time
    /// constant, the most it adds before it is spent).
    pub fn left(&self, goal: f32, e: Ease) -> f32 {
        let carry = if e.curve == Curve::Spring { self.v.abs() * e.secs.max(0.02) / 4.0 } else { 0.0 };
        (goal - self.x).abs() + carry
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME: f32 = 1.0 / 60.0;

    #[test]
    fn every_curve_arrives_and_settles() {
        for (curve, _) in CURVES {
            let e = Ease { curve, secs: 0.32 };
            let mut g = Glider::default();
            g.snap(3.0, 0.0);
            g.retarget(0.0);
            let mut t = 0.0;
            while t < 3.0 {
                t += f64::from(FRAME);
                g.step(5.0, t, FRAME, e);
            }
            assert!((g.x - 5.0).abs() < 0.01 && !g.moving(5.0), "{curve:?} at {}", g.x);
        }
    }

    #[test]
    fn a_spring_is_stable_at_its_shortest_glide_and_after_a_stall() {
        // A word on and 0.6 back, at 60 a second and at a quarter-second frame: never past the goal,
        // never back, and settled.
        for (secs, dt) in [(0.06, FRAME), (0.06, 0.25), (0.32, 0.25), (1.5, 0.25), (0.32, FRAME)] {
            let e = Ease { curve: Curve::Spring, secs };
            for goal in [11.0, 9.4] {
                let mut g = Glider::default();
                g.snap(10.0, 0.0);
                let (mut t, mut last) = (0.0, 10.0f32);
                while t < 4.0 {
                    t += f64::from(dt);
                    let x = g.step(goal, t, dt, e);
                    let on = (x - last) * (goal - 10.0).signum();
                    assert!(x.is_finite() && on >= -1e-6 && (goal - x) * (goal - 10.0).signum() >= -1e-6, "{secs}/{dt} to {goal}: {x}");
                    last = x;
                }
                assert!((g.x - goal).abs() < 0.002 && !g.moving(goal), "{secs}/{dt} to {goal}: {}", g.x);
            }
        }
        // Easing back, then the goal ahead again: it turns at once instead of sliding on back.
        let e = Ease { curve: Curve::Spring, secs: 0.32 };
        let mut g = Glider::default();
        g.snap(10.0, 0.0);
        for k in 1..=6 {
            g.step(9.0, f64::from(k as f32 * FRAME), FRAME, e);
        }
        let x = g.x;
        assert!(x < 10.0 && g.step(10.5, f64::from(7.0 * FRAME), FRAME, e) >= x);
    }

    #[test]
    fn near_the_end_of_the_quran_every_curve_reaches_its_goal_to_the_last_bit() {
        for (curve, _) in CURVES {
            let e = Ease { curve, secs: 0.32 };
            let (mut g, mut f) = (Glider::default(), Glider::default());
            g.snap(73_000.1, 0.0);
            f.snap(73_000.1, 0.0);
            let mut t = 0.0;
            while t < 3.0 {
                t += f64::from(FRAME);
                g.step_exact(73_000.4, t, FRAME, e);
                f.step(73_000.4, t, FRAME, e);
            }
            assert!((g.x - 73_000.4).abs() < 0.005, "{curve:?} at {}", g.x);
            if curve == Curve::Smooth {
                assert!(73_000.4 - f.x > 0.02, "the f32 step stalls short: {}", f.x);
            }
        }
    }

    #[test]
    fn a_glide_is_where_steady_frames_put_it_however_far_apart_the_frames_are() {
        // Frames that come when they are asked for (far apart in a glide's slow end, close in its fast start) or late:
        // stepped by the time since the last, every curve is where 60 or 144 steady frames a second put it at the same
        // moments, to a thousandth of a word. (A goal within two words: further, Smooth eases slower until it is.)
        let at = [0.1, 0.25, 0.5, 0.8, 1.3, 2.0, 3.0];
        let uneven = [0.001, 0.05, 0.0167, 0.12, 0.003, 0.25, 0.0069, 0.09];
        for (curve, _) in CURVES {
            for secs in [0.06, 0.48, 1.5] {
                let e = Ease { curve, secs };
                let glide = |steps: &[f32]| {
                    let mut g = Glider::default();
                    g.snap(10.0, 0.0);
                    g.retarget(0.0);
                    let (mut t, mut k, mut seen) = (0.0f64, 0, Vec::new());
                    for &mark in &at {
                        while t < mark - 1e-9 {
                            let dt = f64::from(steps[k % steps.len()]).min(mark - t);
                            (t, k) = (t + dt, k + 1);
                            g.step(11.5, t, dt as f32, e);
                        }
                        seen.push(g.x);
                    }
                    seen
                };
                let steady = glide(&[1.0 / 60.0]);
                for steps in [&[1.0 / 144.0][..], &uneven] {
                    let other = glide(steps);
                    assert!(steady.iter().zip(&other).all(|(a, b)| (a - b).abs() < 1e-3), "{curve:?} {secs}: {steady:?} vs {other:?}");
                }
            }
        }
        // What is left of a glide: its distance, and a spring's speed carries it on.
        let mut g = Glider::default();
        g.snap(10.0, 0.0);
        let spring = Ease { curve: Curve::Spring, secs: 0.32 };
        g.step(11.0, 0.05, 0.05, spring);
        assert!(g.left(11.0, spring) > 11.0 - g.x && g.left(11.0, Ease { curve: Curve::Smooth, ..spring }) == 11.0 - g.x);
    }

    #[test]
    fn a_glide_that_starts_after_a_hold_starts_smoothly() {
        // Held at 10 for 2 s after the last update (as while waiting for a reader who is behind), then
        // sent back to 9: every curve starts from where it is, Linear too, not from where it would be
        // by now had its glide started at the update.
        for (curve, _) in CURVES {
            let e = Ease { curve, secs: 0.32 };
            let mut g = Glider::default();
            g.snap(10.0, 0.0);
            let mut t = 0.0;
            while t < 2.0 {
                t += f64::from(FRAME);
                g.step(10.0, t, FRAME, e);
            }
            let x = g.step(9.0, t + f64::from(FRAME), FRAME, e);
            assert!(10.0 - x < 0.2, "{curve:?} jumped to {x}");
        }
    }
}
