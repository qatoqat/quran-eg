//! Replaying the newest recording with the voice pointer: the recording plays through the speakers (`sound`) while the
//! installed listener, cued on the same file, hears it at its real pace from the moment its first sample is heard, and
//! the pointer follows it as it follows the reader. The app stops live listening meanwhile and starts it again after.

use std::path::Path;
use std::time::{Duration, Instant};

use quran_core::settings::Voice as Settings;
use quran_core::view::WordRef;

use super::sound::Sound;
use super::{Status, Voice};
use crate::header::{Replaying, clock};

/// Seconds the pointer stays on the last place heard once the recording has played out.
const LINGER: f64 = 1.5;
/// A listener that stops this near the end of its feed (s) has heard the recording out.
const NEAR_END: f32 = 1.0;
/// Seconds a sound that lags behind the listener's feed may go on after it.
const LAG: f32 = 3.0;
/// Seconds the connecting spinner takes to turn once (as the Listen button's).
const SPIN: f64 = 1.6;

/// A replay under way.
pub struct Replay {
    sound: Sound,
    /// The recording's name (its file's stem).
    name: String,
    /// Where it started, as its sidecar says: the word and its page (0 if not said).
    pub start: Option<(WordRef, u16)>,
    /// Live listening was on before: it comes back after.
    pub resume: bool,
    /// When the recording had played out, or its listener had heard it out (UI time).
    ended: Option<f64>,
}

/// What a frame makes of the replay.
#[derive(Debug, PartialEq)]
pub enum Tick {
    Going,
    /// Over: played out, or failed (why, for the title bar).
    Over(Option<String>),
}

impl Replay {
    /// Starts replaying `wav`: its sound, and the voice that follows it (a listener cued on it, which joins
    /// the sound when it is up). `resume`: live listening was on, and comes back after.
    pub fn start(settings: &Settings, ctx: &egui::Context, wav: &Path, resume: bool) -> Result<(Replay, Voice), String> {
        let sound = Sound::start(wav.to_path_buf())?;
        let voice = Voice::replay(settings, ctx, wav, sound.cue.clone())?;
        let name = wav.file_stem().map_or_else(|| wav.display().to_string(), |n| n.to_string_lossy().into_owned());
        let start = crate::recorder::started_at(wav);
        Ok((Replay { sound, name, start, resume, ended: None }, voice))
    }

    /// The Replay button's look.
    pub fn button(&self, now: f64) -> Replaying {
        match self.sound.cue.progress().filter(|_| self.sound.cue.playing()) {
            Some((at, of)) => Replaying::Playing { at, of },
            None => Replaying::Starting((now / SPIN).fract() as f32),
        }
    }

    /// The title bar's line: the recording while it starts; then how far it has played, and what its listener hears.
    pub fn title(&self, voice: Option<&Voice>) -> String {
        let heard = voice.map_or_else(String::new, Voice::replay_summary);
        match self.sound.cue.progress().filter(|_| self.sound.cue.playing()) {
            Some((at, of)) => format!("▶ {} / {} · {heard}", clock(at), clock(of)),
            None => format!("▶ Replay of {} · {heard}", self.name),
        }
    }

    /// Seconds until the listener's feed of the recording ends (negative after): it started with the sound's first
    /// sample heard, and runs at the recording's pace on a clock of its own.
    fn feed_left(&self) -> Option<f32> {
        let (_, length) = self.sound.cue.progress()?;
        let end = self.sound.cue.heard()? + Duration::from_secs_f32(length);
        let now = Instant::now();
        Some(if end >= now { (end - now).as_secs_f32() } else { -(now - end).as_secs_f32() })
    }

    /// Each frame, with the voice following it: whether it goes on. It ends once the sound has played out (or lags too
    /// far behind the listener's end); a listener that stops before the end of its feed, or a sound that fails, ends it
    /// with why. (The listener stops by itself once it has heard the whole recording.)
    pub fn tick(&mut self, voice: Option<&Voice>, now: f64) -> Tick {
        let cue = &self.sound.cue;
        if let Some(e) = cue.error() {
            return Tick::Over(Some(format!("Replay: {e}")));
        }
        let failed = match voice.map(Voice::status) {
            Some(Status::Failed(e)) => Some(e.clone()),
            None => Some("its listener is gone".into()),
            _ => None,
        };
        let (over, left) = (cue.over(), self.feed_left());
        judge(&mut self.ended, over, left, failed, now)
    }
}

/// `tick`'s verdict from what it reads: whether the sound has played out, the seconds left of the listener's feed (none
/// before it starts), why the listener failed if it has; `ended` is when the end began.
fn judge(ended: &mut Option<f64>, over: bool, left: Option<f32>, failed: Option<String>, now: f64) -> Tick {
    if ended.is_none() && (over || left.is_some_and(|l| l < -LAG)) {
        *ended = Some(now);
    }
    match (*ended, failed) {
        (Some(at), _) if now - at >= LINGER => Tick::Over(None),
        (_, Some(e)) if left.is_none_or(|l| l >= NEAR_END) => Tick::Over(Some(format!("Replay: {e}"))),
        _ => Tick::Going,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_replay_ends_once_played_out_and_a_listener_that_stops_early_says_why() {
        let gone = || Some("the listener went away".to_string());
        let mut ended = None;
        // Failing while it starts, or half-way through: over, with why.
        assert_eq!(judge(&mut ended, false, None, gone(), 1.0), Tick::Over(Some("Replay: the listener went away".into())));
        assert_eq!(judge(&mut ended, false, Some(12.0), gone(), 1.0), Tick::Over(Some("Replay: the listener went away".into())));
        assert_eq!(judge(&mut ended, false, Some(12.0), None, 1.0), Tick::Going);
        // The listener stops at the end of its feed; the sound plays out, the pointer stays a moment, and it is over.
        assert_eq!(judge(&mut ended, false, Some(0.3), gone(), 10.0), Tick::Going);
        assert_eq!(judge(&mut ended, true, Some(-0.1), gone(), 10.4), Tick::Going);
        assert_eq!((ended, judge(&mut ended, true, Some(-1.0), gone(), 11.95)), (Some(10.4), Tick::Over(None)));
        // A sound lagging far behind the listener's end is not waited for.
        let mut ended = None;
        assert_eq!(judge(&mut ended, false, Some(-2.9), gone(), 20.0), Tick::Going);
        assert_eq!((judge(&mut ended, false, Some(-3.1), gone(), 20.1), ended), (Tick::Going, Some(20.1)));
        assert_eq!(judge(&mut ended, false, Some(-4.7), gone(), 21.7), Tick::Over(None));
    }
}
