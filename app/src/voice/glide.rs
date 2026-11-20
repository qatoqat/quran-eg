//! One eased value of the voice pointer (the pointer, or an edge of its range): it chases a goal by
//! its own curve and time, so parts with different settings move apart and settle together.

use quran_core::settings::VoiceCurve as Curve;

pub const CURVES: [(Curve, &str); 5] =
    [(Curve::Smooth, "Smooth"), (Curve::Spring, "Spring"), (Curve::Linear, "Linear"), (Curve::EaseOut, "Ease out"), (Curve::EaseInOut, "Ease in-out")];

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
    /// Where the current glide of a time-based curve started, and when.
    from: f32,
    t0: f64,
}

impl Glider {
    /// Put it at `x`, at rest.
    pub fn snap(&mut self, x: f32, now: f64) {
        *self = Glider { x, v: 0.0, from: x, t0: now };
    }

    /// A new place was heard: a time-based curve glides on from here, now.
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
            curve => {
                if goal == self.x {
                    // At rest: the next glide starts here, then (not at the last update, long ago).
                    self.retarget(now);
                } else {
                    let u = (now - self.t0) as f32 / secs;
                    self.x = self.from + (goal - self.from) * ease(curve, u);
                }
            }
        }
        self.x
    }

    /// Still on its way to `goal`.
    pub fn moving(&self, goal: f32) -> bool {
        (goal - self.x).abs() > 0.002 || self.v.abs() > 0.002
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME: f32 = 1.0 / 60.0;

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
    fn a_glide_that_starts_after_a_hold_starts_smoothly() {
        // Held at 10 for 2 s after the last update (as while waiting for a reader who is behind), then
        // sent back to 9: the time-based curves start from where they are, not where they would be by
        // now had they started at the update.
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
