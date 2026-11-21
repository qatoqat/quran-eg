//! Whether the reader is sounding, from the voice levels a listener sends (`{"voice":{"t":..,"db":..}}`, one per 40 ms
//! frame, the number its tracker reads): quran-voice's display presence (`qv_motion::Presence`), in the link thread,
//! which wakes the app only when it changes. The floor is the 10th percentile of the last 10 s and the reader's level is
//! followed over the levels above the middle of that range, so a quiet microphone and a loud one read alike. Present
//! while a level of the last `CARRY` s stood `THR` of the way up; once absent, only `RETURN` such levels in a row bring it
//! back (a click or a breath in a pause is one frame), and a silence ends only once the levels that brought it back
//! span `BLIP` s.

use std::collections::VecDeque;

/// Seconds the floor is taken over, and seconds of levels before which the voice counts as present.
const WINDOW: f64 = 10.0;
const WARMUP: f64 = 3.0;
/// The least floor-to-speech range (dB): on a clean recording the floor is digital silence.
const MIN_RANGE: f32 = 12.0;
/// The voice is present while a level of the last `CARRY` s stood `THR` of the way from the floor to the reader's
/// level: a consonant at the floor is carried, a pause is not.
const THR: f32 = 0.35;
const CARRY: f64 = 0.2;
/// Loud levels in a row that bring an absent voice back, and seconds of them that end a silence.
const RETURN: u32 = 2;
const BLIP: f64 = 0.07;

/// What the presence says after a level: the voice present, and the reader in a silence (from the voice going absent
/// until it is back for `BLIP`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Heard {
    pub present: bool,
    pub quiet: bool,
}

#[derive(Default)]
pub struct Presence {
    /// The last `WINDOW` s of levels (audio time, dB).
    hist: VecDeque<(f64, f32)>,
    sorted: Vec<f32>,
    speech: Option<f32>,
    first: Option<f64>,
    /// Loud levels in a row and when the run began; the last loud level counted, and when the levels that brought the
    /// voice back began.
    run: (u32, f64),
    loud: Option<f64>,
    back: f64,
    heard: Option<Heard>,
}

impl Presence {
    /// A frame's level at audio time `t`; what the presence says if it changed.
    pub fn push(&mut self, t: f32, db: f32) -> Option<Heard> {
        let t = f64::from(t);
        while self.hist.front().is_some_and(|h| t - h.0 >= WINDOW) {
            self.hist.pop_front();
        }
        self.hist.push_back((t, db));
        self.sorted.clear();
        self.sorted.extend(self.hist.iter().map(|h| h.1));
        self.sorted.sort_unstable_by(f32::total_cmp);
        let floor = self.floor();
        let speech = self.speech.get_or_insert(db);
        if db > floor + 0.5 * (speech.max(floor + 1e-6) - floor) {
            *speech += 0.05 * (db - *speech);
        }
        let loud = (db - floor) / (*speech - floor).max(MIN_RANGE) > THR;
        let warm = t - *self.first.get_or_insert(t) < WARMUP;
        self.run = if !loud { (0, t) } else if self.run.0 == 0 { (1, t) } else { (self.run.0 + 1, self.run.1) };
        let was = self.heard.unwrap_or(Heard { present: false, quiet: false });
        if loud && (was.present || warm || self.run.0 >= RETURN) {
            self.loud = Some(t);
        }
        let present = warm || self.loud.is_some_and(|l| t - l < CARRY);
        let mut quiet = was.quiet || (was.present && !present);
        if present && !was.present {
            self.back = self.run.1;
        }
        if present && self.loud.is_some_and(|l| l - self.back >= BLIP) {
            quiet = false;
        }
        let now = Heard { present, quiet };
        (self.heard != Some(now)).then(|| {
            self.heard = Some(now);
            now
        })
    }

    fn floor(&self) -> f32 {
        let k = 0.1 * (self.sorted.len() - 1) as f32;
        let (i, frac) = (k as usize, k.fract());
        self.sorted[i] + frac * (self.sorted[(i + 1).min(self.sorted.len() - 1)] - self.sorted[i])
    }

    /// The audio time (s) of the last loud level: where a silence began.
    pub fn last_loud(&self) -> Option<f32> {
        self.loud.map(|t| t as f32)
    }

    /// How far the reader's level stands over the room's (dB), once the floor has the warm-up behind it.
    pub fn range(&self) -> Option<f32> {
        let (first, last) = (self.first?, self.hist.back()?.0);
        (last - first >= WARMUP).then(|| self.speech.map(|s| s - self.floor())).flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 12 s of reading (a consonant at the floor in every third frame), then a pause; `loud` frames of it loud. What the
    /// presence said when it changed, by frame.
    fn read(loud: impl Fn(usize) -> bool) -> (Presence, Vec<(usize, Heard)>) {
        let mut p = Presence::default();
        let changes = (0..400).filter_map(|f| p.push(f as f32 * 0.04, if (f < 300 && f % 3 != 0) || loud(f) { -30.0 } else { -52.0 }).map(|h| (f, h))).collect();
        (p, changes)
    }

    const fn heard(present: bool, quiet: bool) -> Heard {
        Heard { present, quiet }
    }

    #[test]
    fn it_changes_with_a_pause_once_its_carry_is_over_and_with_the_voice_back() {
        // The first frame's news, the pause once a carry has passed the last loud frame (299), the voice back at the
        // second frame of the reader again (350) and the silence over once they span 0.07 s.
        let (p, changes) = read(|f| f >= 350);
        assert_eq!(changes.len(), 4, "{changes:?}");
        assert!(changes[0] == (0, heard(true, false)) && (303..=305).contains(&changes[1].0) && changes[1].1 == heard(false, true), "{changes:?}");
        assert_eq!(&changes[2..], [(351, heard(true, true)), (352, heard(true, false))], "{changes:?}");
        assert!(p.range().is_some_and(|r| (r - 22.0).abs() < 2.0), "{:?}", p.range());
    }

    #[test]
    fn a_click_does_not_bring_the_voice_back_nor_a_short_return_end_the_silence() {
        let (_, changes) = read(|f| [340, 360, 361].contains(&f));
        assert_eq!(changes.len(), 4, "{changes:?}");
        assert!(changes[2] == (361, heard(true, true)) && (365..=367).contains(&changes[3].0) && changes[3].1 == heard(false, true), "{changes:?}");
    }
}
