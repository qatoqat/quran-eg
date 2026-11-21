//! A microphone that hears the voice only weakly (a hanging earphone mic; held close to the mouth it works): the voice
//! stands little over the room's sound, and the listener jumps about and loses the place. The microphone's tooltip
//! says so, but a reader reciting never sees it, so after a few seconds of weak speech a toast says it, until the
//! voice is strong again, and not again for a while; dismissed, not again while listening. Only speech counts: the
//! reader's pauses are not weak.

/// The voice's stand over the room's sound (dB) below which it is weak, and above which it is strong again. On the
/// reader's recordings (the median while voiced): a hanging earphone microphone 11 dB, one far off 13.5, held close
/// 16-22 (the floor is a low share of the levels, some of them speech, so a close voice stands no more than this).
pub const WEAK: f32 = 12.0;
const STRONG: f32 = 15.0;
/// Seconds of weak speech before the toast shows, and the least time between two toasts.
const AFTER: f64 = 5.0;
const AGAIN: f64 = 90.0;

#[derive(Debug)]
pub struct Weak {
    /// The voice present, and its stand over the room's sound, as last heard; when that was taken in.
    present: bool,
    range: Option<f32>,
    at: f64,
    /// Seconds of weak speech since the voice was last strong; whether the toast shows, and since when.
    weak: f64,
    shown: Option<f64>,
    last: f64,
    /// Dismissed by the reader: not shown again while listening.
    dismissed: bool,
}

impl Default for Weak {
    fn default() -> Self {
        Weak { present: false, range: None, at: 0.0, weak: 0.0, shown: None, last: f64::NEG_INFINITY, dismissed: false }
    }
}

impl Weak {
    /// The voice came or went (`present`), standing `range` dB over the room's sound, at clock time `now`.
    pub fn voice(&mut self, present: bool, range: Option<f32>, now: f64) {
        self.tick(now);
        (self.present, self.range) = (present, range.or(self.range));
        if present && self.range.is_some_and(|r| r >= STRONG) {
            (self.weak, self.shown) = (0.0, None);
        }
    }

    /// The reader dismissed the toast.
    pub fn dismiss(&mut self) {
        (self.shown, self.dismissed) = (None, true);
    }

    /// Whether the toast shows at clock time `now`.
    pub fn tick(&mut self, now: f64) -> bool {
        if self.dismissed {
            return false;
        }
        if self.present && self.range.is_some_and(|r| r < WEAK) {
            self.weak += (now - self.at).max(0.0);
        }
        self.at = now;
        if self.shown.is_none() && self.weak >= AFTER && now - self.last >= AGAIN {
            (self.shown, self.last) = (Some(now), now);
        }
        self.shown.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Speech at `range` dB for `secs` from `t`, in syllables of 0.4 s with 0.1 s gaps; whether the toast showed by the
    /// end.
    fn speak(w: &mut Weak, range: f32, t: f64, secs: f64) -> bool {
        let mut now = t;
        while now < t + secs {
            w.voice(true, Some(range), now);
            w.tick(now + 0.2);
            w.voice(false, Some(range), now + 0.4);
            now += 0.5;
        }
        w.tick(now)
    }

    #[test]
    fn weak_speech_for_some_seconds_shows_the_toast_until_the_voice_is_strong() {
        let mut w = Weak::default();
        assert!(!speak(&mut w, 9.0, 0.0, 4.0), "not yet");
        assert!(speak(&mut w, 9.0, 4.0, 3.0), "after about 5 s of weak speech");
        assert!(!speak(&mut w, 20.0, 7.0, 0.5), "gone once the voice is strong");
        // Weak again soon after: not again within the minute and a half.
        assert!(!speak(&mut w, 9.0, 8.0, 30.0));
        assert!(speak(&mut w, 9.0, 100.0, 7.0), "and again after it");
    }

    #[test]
    fn a_close_voice_never_shows_it_and_a_dismissed_toast_stays_away() {
        // Held close, a voice stands 16-22 dB over the room on the reader's recordings.
        let mut w = Weak::default();
        assert!(!speak(&mut w, 17.0, 0.0, 60.0));
        assert!(speak(&mut w, 9.0, 60.0, 7.0));
        w.dismiss();
        assert!(!w.tick(67.1));
        assert!(!speak(&mut w, 9.0, 200.0, 30.0), "not again while listening");
    }

    #[test]
    fn pauses_and_a_strong_voice_never_show_it() {
        let mut w = Weak::default();
        assert!(!speak(&mut w, 32.0, 0.0, 20.0));
        // A long pause with a weak level standing: no speech, nothing counted.
        w.voice(false, Some(10.0), 20.0);
        assert!(!w.tick(40.0));
    }
}
