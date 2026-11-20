//! The microphone's meter on the Listen button. The listener measures its input about 15 times a
//! second; the button's disc fills from the bottom as high as the voice is loud, rising quickly and
//! settling slowly so that it bobs calmly between syllables. Clipping turns it the danger colour for a
//! moment; the listener's word that no audio arrives, or that capture failed, strikes it through. It
//! asks for frames only while it changes.

use super::link::{Audio, Device, Level};
use crate::kit::Mic;

/// The input levels (dBFS) shown empty and full: room noise stays out, a loud voice fills the disc.
const FLOOR: f32 = -54.0;
const FULL: f32 = -6.0;
/// From this level up (near a fifth of the disc) the tooltip says it hears you; below, the input is
/// quiet, as steady room noise over the floor is.
const VOICE: f32 = -45.0;
/// A peak this near full scale is too loud (clipping, or about to).
const LOUD_PEAK: f32 = -1.0;
/// An RMS this low is digital silence: a muted input.
const SILENCE: f32 = -90.0;
/// Seconds the danger colour holds after the last loud level, and then fades over.
const HOLD: f64 = 1.5;
const FADE: f64 = 0.4;
/// Time constants of the fill's rise and fall (s), and its slowest fall (of the disc a second), so
/// that it lands instead of creeping.
const ATTACK: f32 = 0.08;
const RELEASE: f32 = 0.6;
const MIN_FALL: f32 = 0.08;
/// Within this of its level the fill is at rest (a seventh of a point).
const REST: f32 = 0.004;
/// The least change of level worth waking the app for: about a point of the disc (1.4 dB); below the
/// floor, where nothing moves, 6 dB (the tooltip's number, digital silence).
const STEP: f32 = 0.03;
const QUIET_STEP: f32 = 6.0;
/// The tooltip says the loudest level of each half second, and holds it for the next, so that it can
/// be read.
const SAY: f64 = 0.5;
/// Seconds between frames while the fill moves: plenty for a slow, faint tint.
pub const FRAME: f32 = 1.0 / 30.0;

/// An input level (dBFS) as the share of the disc it fills: linear in dB from the floor to full.
fn display(db: f32) -> f32 {
    ((db - FLOOR) / (FULL - FLOOR)).clamp(0.0, 1.0)
}

/// The share of the disc a level fills after one that filled `was`. An empty disc fills only from
/// three steps over the floor and empties again under one, so that noise wavering there leaves it be.
fn fill(rms: f32, was: f32) -> f32 {
    let d = display(rms);
    let least = if was > 0.0 { STEP } else { 3.0 * STEP };
    if d < least { 0.0 } else { d }
}

fn loud(l: &Level) -> bool {
    l.clip || l.peak >= LOUD_PEAK
}

/// The link thread's deadband: a level wakes the app only if it is the first since the connection or
/// trouble, is loud, or moves the fill (or the quiet number); so a quiet room or steady noise wakes
/// nothing. It fills the disc level by level as the meter does, which hears just what it passes.
#[derive(Default)]
pub struct Gate {
    /// The last level passed on: its fill and RMS.
    last: Option<(f32, f32)>,
}

impl Gate {
    pub fn pass(&mut self, audio: &Audio) -> bool {
        let l = match audio {
            Audio::Level(l) => l,
            // Trouble empties the meter: the next level shows afresh.
            Audio::Stalled | Audio::Error(_) => {
                self.last = None;
                return true;
            }
            Audio::Mic(_) => return true,
        };
        let d = fill(l.rms, self.last.map_or(0.0, |(d, _)| d));
        // With the disc empty, only the tooltip's number moves.
        let moved = |(last, rms): (f32, f32)| if d == 0.0 && last == 0.0 { (l.rms - rms).abs() >= QUIET_STEP } else { (d - last).abs() >= STEP };
        let pass = loud(l) || self.last.is_none_or(moved);
        if pass {
            self.last = Some((d, l.rms));
        }
        pass
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Trouble {
    Stalled,
    Error(String),
}

/// The meter of one connection to the listener.
#[derive(Default)]
pub struct Meter {
    device: Option<Device>,
    /// The fill drawn (0..=1) and the fill it moves to.
    shown: f32,
    target: f32,
    /// The newest level's fill, if one came since the last frame.
    fresh: Option<f32>,
    /// The newest level's RMS and peak (dBFS), and the loudest since the tooltip last said one.
    level: Option<(f32, f32)>,
    loudest: Option<(f32, f32)>,
    /// What the tooltip says, and since when.
    said: Option<(f64, (f32, f32))>,
    /// When the last loud level came.
    loud_at: Option<f64>,
    trouble: Option<Trouble>,
    /// When the fill last moved; `None` at rest.
    moved: Option<f64>,
}

/// One step of the fill from `shown` toward `level` over `dt` seconds.
fn settle(shown: f32, level: f32, dt: f32) -> f32 {
    let gap = level - shown;
    let step = if gap > 0.0 {
        gap * (1.0 - (-dt / ATTACK).exp())
    } else {
        (gap * (1.0 - (-dt / RELEASE).exp())).min(-MIN_FALL * dt).max(gap)
    };
    let next = shown + step;
    if (level - next).abs() < REST { level } else { next }
}

/// Whole decibels, with an ASCII minus (and no "-0").
fn db(x: f32) -> String {
    format!("{:.0}", x.round() + 0.0)
}

/// "HDA Intel PCH, 48 kHz stereo"; what the listener did not say is left out.
fn describe(d: &Device) -> String {
    let rate = (d.rate > 0).then(|| format!("{} kHz", f64::from(d.rate) / 1000.0));
    let channels = match d.channels {
        0 => None,
        1 => Some("mono".to_string()),
        2 => Some("stereo".to_string()),
        n => Some(format!("{n} channels")),
    };
    let format = [rate, channels].into_iter().flatten().collect::<Vec<_>>().join(" ");
    [d.name.trim().to_string(), format].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(", ")
}

impl Meter {
    /// News of the microphone, at clock time `now`.
    pub fn hear(&mut self, audio: Audio, now: f64) {
        match audio {
            Audio::Mic(device) => self.device = Some(device),
            Audio::Level(l) => {
                // Audio again: any trouble is over (and left the fill empty).
                self.trouble = None;
                // The newest level sets the fill, as it does the gate's (a frame after the window was
                // hidden hears a backlog).
                self.fresh = Some(fill(l.rms, self.fresh.unwrap_or(self.target)));
                self.level = Some((l.rms, l.peak));
                self.loudest = Some(self.loudest.map_or((l.rms, l.peak), |(r, p)| (r.max(l.rms), p.max(l.peak))));
                if loud(&l) {
                    self.loud_at = Some(now);
                }
            }
            // No audio says less than a capture error.
            Audio::Stalled if matches!(self.trouble, Some(Trouble::Error(_))) => {}
            Audio::Stalled => self.fail(Trouble::Stalled),
            Audio::Error(e) => self.fail(Trouble::Error(e)),
        }
    }

    /// Trouble with the microphone: the levels heard so far no longer stand.
    fn fail(&mut self, trouble: Trouble) {
        *self = Meter { device: self.device.take(), trouble: Some(trouble), ..Meter::default() };
    }

    /// Moves the fill toward the level heard, and says the tooltip's level anew each half second.
    /// Returns how soon it wants another frame: soon while the fill moves or its colour fades, at the
    /// end of a loud level's hold or of the half second if the level has changed, never at rest.
    pub fn tick(&mut self, now: f64) -> Option<f32> {
        if let Some(fresh) = self.fresh.take() {
            self.target = fresh;
        }
        // The first step from rest is a frame's: the level came just now.
        let dt = self.moved.map_or(FRAME, |t| (now - t) as f32).clamp(0.0, 0.25);
        self.shown = settle(self.shown, self.target, dt);
        let moving = self.shown != self.target;
        self.moved = moving.then_some(now);
        if let Some(loudest) = self.loudest
            && self.said.is_none_or(|(t, _)| now - t >= SAY)
        {
            self.said = Some((now, loudest));
            // The newest level carries on into the next half second.
            self.loudest = self.level;
        }
        // Said again when the half second is over, if the newest level is not what it says.
        let say = self.said.filter(|&(_, said)| self.level != Some(said)).map(|(t, _)| (t + SAY - now) as f32);
        let hold = self.loud_at.map_or(f64::NEG_INFINITY, |t| t + HOLD - now);
        if moving || (-FADE..=0.0).contains(&hold) {
            Some(FRAME)
        } else {
            // No sooner than a frame: one that comes a hair early would otherwise ask for a burst.
            [(hold > 0.0).then_some(hold as f32), say].into_iter().flatten().reduce(f32::min).map(|after| after.max(FRAME))
        }
    }

    /// The button while listening.
    pub fn mic(&self, now: f64) -> Mic {
        if self.trouble.is_some() { Mic::Trouble } else { Mic::On { level: self.shown, loud: self.loud(now) } }
    }

    /// How far the button has turned the danger colour: 1 while a loud level holds, then fading.
    fn loud(&self, now: f64) -> f32 {
        self.loud_at.map_or(0.0, |t| (1.0 - (now - t - HOLD).max(0.0) / FADE).max(0.0) as f32)
    }

    /// The microphone's trouble, for the title bar.
    pub fn trouble(&self) -> Option<String> {
        Some(match self.trouble.as_ref()? {
            Trouble::Stalled => "no audio from the microphone".into(),
            Trouble::Error(e) => format!("microphone error: {e}"),
        })
    }

    /// The tooltip's lines on the microphone (how it hears, then which it is), once the listener has
    /// said anything about it. The name has a line of its own, so that the level's stays short.
    pub fn line(&self, now: f64) -> Option<String> {
        let device = self.device.as_ref().map(describe).filter(|d| !d.is_empty());
        let what = match (&self.trouble, self.said) {
            (Some(Trouble::Stalled), _) => "Microphone: no audio is arriving. Is it plugged in, or in use by another program?".into(),
            (Some(Trouble::Error(e)), _) => format!("Microphone error: {e}"),
            (None, None) => return device.map(|d| format!("Microphone: {d}")),
            (None, Some(_)) if self.loud(now) > 0.0 => "Microphone: too loud, the sound is clipping. Move back or lower the input volume".into(),
            (None, Some((_, (rms, _)))) if rms <= SILENCE => "Microphone: no sound at all, is it muted?".into(),
            (None, Some((_, (rms, peak)))) if rms >= VOICE => format!("Microphone: hearing you, {} dB (peak {} dB)", db(rms), db(peak)),
            (None, Some((_, (rms, _)))) => format!("Microphone: quiet ({} dB)", db(rms)),
        };
        Some(match device {
            Some(d) => format!("{what}\n{d}"),
            None => what,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A steady tone.
    fn level(rms: f32) -> Audio {
        Audio::Level(Level { rms, peak: rms, clip: false })
    }

    /// A voice: its peaks 12 dB over its RMS.
    fn voice(rms: f32) -> Audio {
        Audio::Level(Level { rms, peak: rms + 12.0, clip: false })
    }

    /// Ticks at 30 frames a second from `t` while the meter asks for the next frame; returns when it
    /// stops.
    fn settle_from(m: &mut Meter, mut t: f64) -> f64 {
        while m.tick(t).is_some_and(|after| after <= FRAME) {
            t += f64::from(FRAME);
            assert!(t < 100.0, "the meter comes to rest");
        }
        t
    }

    /// What the tooltip says of the level, without the device.
    fn says(m: &Meter, now: f64) -> String {
        m.line(now).unwrap().lines().next().unwrap().to_string()
    }

    /// The link thread's gate and the meter, as the app runs them.
    fn through(gate: &mut Gate, m: &mut Meter, audio: Audio, now: f64) {
        if gate.pass(&audio) {
            m.hear(audio, now);
        }
    }

    #[test]
    fn levels_fill_the_disc_linearly_in_db_over_a_margin() {
        assert_eq!([-54.0, -6.0, -30.0, -120.0, 3.0].map(display), [0.0, 1.0, 0.5, 0.0, 1.0]);
        assert_eq!((fill(-50.0, 0.0), fill(-30.0, 0.0)), (0.0, 0.5), "an empty disc fills from three steps up");
        assert!(fill(-49.5, 0.0) > 0.09);
        assert!(fill(-52.0, 0.5) > 0.04, "and empties under one");
        assert_eq!(fill(-53.0, 0.5), 0.0);
    }

    #[test]
    fn the_fill_rises_quickly_falls_slowly_and_rests() {
        let near = |a: f32, b: f32| (a - b).abs() < 0.01;
        assert!(near(settle(0.0, 1.0, 0.08), 0.632));
        assert!(near((0..5).fold(0.0, |s, _| settle(s, 1.0, 0.016)), 0.632), "the same at any frame rate");
        assert!(near((0..36).fold(1.0, |s, _| settle(s, 0.0, 1.0 / 60.0)), 0.368));
        let mut m = Meter::default();
        m.hear(level(-6.0), 0.0);
        let full = settle_from(&mut m, 0.0);
        assert_eq!(m.mic(full), Mic::On { level: 1.0, loud: 0.0 });
        m.hear(level(-70.0), full);
        let empty = settle_from(&mut m, full);
        assert!(empty - full < 3.0, "empty within 3 s, not {}", empty - full);
        assert_eq!(m.mic(empty), Mic::On { level: 0.0, loud: 0.0 });
        for k in 1..100 {
            assert_eq!(m.tick(empty + f64::from(k) * 0.5), None, "at rest, the meter asks for no frames");
        }
    }

    #[test]
    fn a_loud_level_holds_the_danger_colour_then_fades() {
        let mut m = Meter::default();
        m.hear(Audio::Level(Level { rms: -20.0, peak: -8.0, clip: true }), 10.0);
        assert!((0..=150).all(|k| m.loud(10.0 + f64::from(k) * 0.01) == 1.0));
        let fade: Vec<f32> = (150..=192).map(|k| m.loud(10.0 + f64::from(k) * 0.01)).collect();
        assert!(fade.windows(2).all(|w| w[1] <= w[0]) && fade[25] < 0.5 && fade[42] == 0.0);
        settle_from(&mut m, 10.0);
        m.hear(Audio::Level(Level { rms: -20.0, peak: -8.0, clip: true }), 11.0);
        assert_eq!(m.loud(12.4), 1.0, "another clip holds it longer");
        assert_eq!(m.tick(12.0), Some(0.5), "a frame at the end of the hold, none before");
        let mut m = Meter::default();
        m.hear(Audio::Level(Level { rms: -20.0, peak: -0.5, clip: false }), 0.0);
        assert_eq!(m.loud(0.0), 1.0, "a peak near full scale is too loud");
        let mut m = Meter::default();
        m.hear(Audio::Level(Level { rms: -20.0, peak: -3.0, clip: false }), 0.0);
        assert_eq!(m.loud(0.0), 0.0);
    }

    #[test]
    fn the_gate_passes_only_levels_that_move_the_meter() {
        let mut gate = Gate::default();
        assert!(gate.pass(&level(-30.0)), "the first level");
        assert!(!gate.pass(&level(-29.04)), "+0.02 of the disc");
        assert!(gate.pass(&level(-28.5)), "+0.03");
        assert!(gate.pass(&level(-53.0)), "down into the margin: empty");
        assert!(!gate.pass(&level(-58.0)), "below the floor, nothing moves");
        assert!(!gate.pass(&level(-50.0)), "nor in the margin over it");
        assert!(gate.pass(&level(-120.0)), "but the quiet number does, by 6 dB");
        assert!(gate.pass(&Audio::Level(Level { rms: -120.0, peak: -0.5, clip: true })), "loud, always");
        assert!(gate.pass(&Audio::Stalled) && gate.pass(&level(-120.0)), "the first level after trouble");
        assert!(gate.pass(&Audio::Mic(Device::default())) && !gate.pass(&level(-120.0)), "news of the device moves nothing");
    }

    #[test]
    fn noise_at_the_floor_wakes_nothing() {
        let jitter = |k: u32| ((k * 37 + 11) % 41) as f32 / 20.0 - 1.0;
        for (centre, half) in [(FLOOR, 0.5), (FLOOR, 1.5), (FLOOR + 1.5, 1.5), (-45.0, 0.5)] {
            let mut gate = Gate::default();
            let passed = (0..300).filter(|&k| gate.pass(&level(centre + half * jitter(k)))).count();
            assert_eq!(passed, 1, "{centre} ± {half} dB: the first level only");
        }
    }

    #[test]
    fn a_frame_that_hears_a_backlog_follows_its_newest_level() {
        // The reader recites, and stops while no frames run (the window hidden): the next frame hears
        // their last loud levels and the quiet room at once.
        let (mut gate, mut m) = (Gate::default(), Meter::default());
        for (k, rms) in [-20.0, -24.0, -19.0].into_iter().enumerate() {
            through(&mut gate, &mut m, voice(rms), k as f64 / 15.0);
            m.tick(k as f64 / 15.0);
        }
        for rms in [-18.5, -21.9, -61.1, -62.0, -60.5, -63.0] {
            through(&mut gate, &mut m, voice(rms), 6.0);
        }
        let rest = settle_from(&mut m, 6.0);
        assert_eq!(m.mic(rest), Mic::On { level: 0.0, loud: 0.0 }, "the disc empties");
        assert_eq!(says(&m, rest), "Microphone: quiet (-61 dB)");
        for rms in [-62.0, -60.5, -61.5] {
            through(&mut gate, &mut m, voice(rms), rest + 1.0);
        }
        assert_eq!(m.tick(rest + 1.0), None, "the quiet room wakes nothing more");
    }

    #[test]
    fn trouble_strikes_the_microphone_until_audio_comes() {
        let mut m = Meter::default();
        m.hear(level(-30.0), 0.0);
        settle_from(&mut m, 0.0);
        m.hear(Audio::Stalled, 1.0);
        assert_eq!((m.mic(1.0), m.tick(1.0)), (Mic::Trouble, None));
        assert_eq!(m.trouble().as_deref(), Some("no audio from the microphone"));
        m.hear(level(-30.0), 2.0);
        assert_eq!(m.mic(2.0), Mic::On { level: 0.0, loud: 0.0 }, "the fill starts from empty");
        m.tick(2.0);
        assert!(matches!(m.mic(2.0), Mic::On { level, .. } if level > 0.0 && level < 0.5));
        m.hear(Audio::Error("unplugged".into()), 3.0);
        m.hear(Audio::Stalled, 4.0);
        assert_eq!(m.trouble().as_deref(), Some("microphone error: unplugged"), "no audio says less");
        assert_eq!(Meter::default().line(0.0), None, "a new connection knows nothing yet");
    }

    #[test]
    fn the_tooltip_says_the_loudest_level_of_each_half_second() {
        let mut m = Meter::default();
        let at = |m: &mut Meter, rms: f32, t: f64| {
            m.hear(voice(rms), t);
            m.tick(t)
        };
        at(&mut m, -30.0, 0.0);
        assert_eq!(says(&m, 0.0), "Microphone: hearing you, -30 dB (peak -18 dB)");
        at(&mut m, -22.0, 0.1);
        at(&mut m, -36.0, 0.2);
        at(&mut m, -31.0, 0.3);
        assert_eq!(says(&m, 0.3), "Microphone: hearing you, -30 dB (peak -18 dB)", "held for half a second");
        m.tick(0.5);
        assert_eq!(says(&m, 0.5), "Microphone: hearing you, -22 dB (peak -10 dB)");
        // The reader stops: the half second after the last syllable says it, the next the quiet.
        at(&mut m, -61.0, 0.6);
        m.tick(1.0);
        assert_eq!(says(&m, 1.0), "Microphone: hearing you, -31 dB (peak -19 dB)");
        m.tick(1.5);
        assert_eq!(says(&m, 1.5), "Microphone: quiet (-61 dB)");
        // Steady noise over the floor is no voice.
        let mut m = Meter::default();
        m.hear(level(-48.0), 0.0);
        assert_eq!(m.tick(0.0), Some(FRAME));
        assert_eq!(says(&m, 0.0), "Microphone: quiet (-48 dB)");
        // A mute, with the disc empty: no frame but the ones to say it.
        let mut m = Meter::default();
        m.hear(level(-60.0), 0.0);
        assert_eq!(m.tick(0.0), None);
        m.hear(level(-120.0), 0.2);
        assert_eq!(m.tick(0.2), Some(0.3), "a frame when the half second is over");
        assert_eq!(m.tick(0.5), Some(0.5), "which says the loudest of it, and another");
        assert_eq!(says(&m, 0.5), "Microphone: quiet (-60 dB)");
        assert_eq!(m.tick(1.0), None);
        assert_eq!(says(&m, 1.0), "Microphone: no sound at all, is it muted?");
    }

    #[test]
    fn the_tooltip_says_what_the_microphone_hears() {
        let mut m = Meter::default();
        assert_eq!(m.line(0.0), None, "nothing until the listener says something");
        m.hear(level(-58.0), 0.0);
        m.tick(0.0);
        assert_eq!(m.line(0.0).as_deref(), Some("Microphone: quiet (-58 dB)"));
        m.hear(Audio::Mic(Device { name: "HDA Intel PCH".into(), rate: 48000, channels: 2 }), 0.0);
        assert_eq!(m.line(0.0).as_deref(), Some("Microphone: quiet (-58 dB)\nHDA Intel PCH, 48 kHz stereo"));
        m.hear(level(-120.0), 1.0);
        m.tick(1.0);
        m.tick(1.5);
        assert!(m.line(1.5).unwrap().starts_with("Microphone: no sound at all, is it muted?\n"));
        m.hear(Audio::Level(Level { rms: -30.0, peak: -9.2, clip: false }), 2.0);
        m.tick(2.0);
        assert!(m.line(2.0).unwrap().starts_with("Microphone: hearing you, -30 dB (peak -9 dB)\n"));
        m.hear(Audio::Level(Level { rms: -10.0, peak: 0.0, clip: true }), 2.1);
        assert!(m.line(2.1).unwrap().starts_with("Microphone: too loud, the sound is clipping. Move back or lower the input volume\n"));
        m.hear(Audio::Stalled, 2.2);
        assert!(m.line(2.2).unwrap().starts_with("Microphone: no audio is arriving. Is it plugged in, or in use by another program?\n"));
        m.hear(Audio::Error("unplugged".into()), 2.3);
        assert_eq!(m.line(2.3).as_deref(), Some("Microphone error: unplugged\nHDA Intel PCH, 48 kHz stereo"));
        let mut m = Meter::default();
        m.hear(Audio::Mic(Device { name: String::new(), rate: 44100, channels: 1 }), 0.0);
        assert_eq!(m.line(0.0).as_deref(), Some("Microphone: 44.1 kHz mono"));
    }
}
