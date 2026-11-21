//! What the settings' preview plays: al-Fatiha's last two ayat on three lines, a reader who recites them (where
//! they really are, the truth) and a listener who hears them (what quran-voice would send), in six scenes that
//! each put some of the settings to work and loop on their own. The listener decides every 0.32 s and sends only
//! changes; a perfect one (`Even`) sends every step alike, the real one (`Heard`, measured on replays of the
//! user's own recordings) in bursts: a word held, then most of one at once, a step back at each breath. Where a
//! setting is a time (a wait, a fade's delay) the reader holds on for as long as it takes to be seen.

use quran_core::settings::Voice;

use super::link::{Heard, Loc};
use super::motion::{Plan, UNITS_PER_WORD};
use super::presence;

/// Seconds between the listener's decisions.
pub const TICK: f64 = 0.32;
/// The reader's pace, words a second (replays of real recitations: 0.8 to 1.2).
const PACE: f32 = 1.1;
/// Index in the whole Quran of 1:6:1 (1:1 to 1:5 have 17 words).
pub const FIRST: u32 = 17;
/// Letters of a word for the perfect listener: fine enough that its every step is alike.
const FINE: u16 = 10_000;
/// Seconds a time setting's outcome stays in sight before the reader goes on.
const SEEN: f32 = 1.6;

/// A word of the passage, or an ayah's end.
pub struct Glyph {
    pub loc: Loc,
    /// Its glyph in the page-1 fonts (QCF V2 and V4 alike), and its text, which the Hafs face draws without them.
    pub code: char,
    pub text: &'static str,
    /// Its base letters (the listener's unit); 0 for an ayah's end.
    pub letters: u16,
    /// Its width in em in the page-1 font: the box it has before a font is there.
    pub advance: f32,
}

const fn glyph(loc: Loc, code: char, text: &'static str, letters: u16, advance: f32) -> Glyph {
    Glyph { loc, code, text, letters, advance }
}

/// 1:6 and 1:7 on three lines, each in reading order: 1:6; 1:7's first five words; its last four.
pub const LINES: [&[Glyph]; 3] = [
    &[
        glyph((1, 6, 1), '\u{FC57}', "ٱهْدِنَا", 5, 2.62),
        glyph((1, 6, 2), '\u{FC58}', "ٱلصِّرَٰطَ", 5, 2.42),
        glyph((1, 6, 3), '\u{FC59}', "ٱلْمُسْتَقِيمَ", 8, 3.35),
        glyph((1, 6, 4), '\u{FC5A}', "٦", 0, 1.0),
    ],
    &[
        glyph((1, 7, 1), '\u{FC5B}', "صِرَٰطَ", 3, 1.95),
        glyph((1, 7, 2), '\u{FC5C}', "ٱلَّذِينَ", 5, 1.56),
        glyph((1, 7, 3), '\u{FC5D}', "أَنْعَمْتَ", 5, 2.27),
        glyph((1, 7, 4), '\u{FC5E}', "عَلَيْهِمْ", 5, 2.05),
        glyph((1, 7, 5), '\u{FC5F}', "غَيْرِ", 3, 1.31),
    ],
    &[
        glyph((1, 7, 6), '\u{FC60}', "ٱلْمَغْضُوبِ", 7, 4.09),
        glyph((1, 7, 7), '\u{FC61}', "عَلَيْهِمْ", 5, 2.42),
        glyph((1, 7, 8), '\u{FC62}', "وَلَا", 3, 1.12),
        glyph((1, 7, 9), '\u{FC63}', "ٱلضَّآلِّينَ", 7, 3.70),
        glyph((1, 7, 10), '\u{FC64}', "٧", 0, 1.0),
    ],
];

/// The passage's words (not its ayah ends), in reading order: word `i` is at position `i` (words into it).
pub fn words() -> impl Iterator<Item = &'static Glyph> {
    LINES.iter().flat_map(|line| line.iter()).filter(|g| g.letters > 0)
}

/// The scenes, each starting with what it needs to show its part (a place followed for a while, a pace).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scene {
    Read,
    AyahEnd,
    Hold,
    Slips,
    Misheard,
    Far,
}

impl Scene {
    pub const ALL: [Scene; 6] = [Scene::Read, Scene::AyahEnd, Scene::Hold, Scene::Slips, Scene::Misheard, Scene::Far];

    pub fn label(self) -> &'static str {
        match self {
            Scene::Read => "Read",
            Scene::AyahEnd => "Ayah end",
            Scene::Hold => "Hold",
            Scene::Slips => "Slips",
            Scene::Misheard => "Misheard",
            Scene::Far => "Far",
        }
    }

    /// What the scene shows at work, and what a few lines cannot.
    pub fn hint(self) -> &'static str {
        match self {
            Scene::Read => "All three lines read on, with a breath at the ayah's end: the curves and times of the pointer and the range's edges, the range's and the dot's look, smudge, shrink and lift. Even against As heard tells the listener's stutter from the settings'.",
            Scene::AyahEnd => "A pause at an ayah's end as long as \"fade out after\" needs: fading out, and fading in where the reader goes on.",
            Scene::Hold => "A letter held sends nothing new: the pointer runs on at the reader's pace up to \"run ahead at most\" (for a second at most), and waits when the next news finds it ahead; with tajweed timing it creeps across the letter and waits at its end. With the voice's level, a breath inside the ayah shows the hold and the dim.",
            Scene::Slips => "A word repeated, 0.85 back (it fades over below 0.9), and held as long as \"wait before easing back\" needs; 4 words skipped (it fades over at 3 or 4); a step back of 4.5 words over a line (it fades over below about 4, else the pointer waits and the range reaches back).",
            Scene::Misheard => "Heard for a second in another surah: lost, the page stays, and back it counts as a slip. With no wait the reader would be taken there.",
            Scene::Far => "Reading on in another surah: the reader is taken there after \"wait before following to a far place\"; never without \"keep the line in view\".",
        }
    }
}

/// A place off the passage: its first word (location, and index in the whole Quran), how far into it it is first
/// heard, how fast it goes on (words a second) and how far at most, and how sure the listener is of it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Place {
    pub loc: Loc,
    pub word: u32,
    pub from: f32,
    pub pace: f32,
    most: f32,
    conf: f32,
}

/// What the listener hears of a stretch of the recitation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Ear {
    /// The reader has not begun.
    Nothing,
    /// The reader reads on, heard at the cadence; unsure (Tentative) at first.
    Reading { sure: bool },
    /// The reader holds a letter (nothing new to send) or pauses (the listener says so, once).
    Still { paused: bool },
    /// Heard at another place: a mishearing, or the reader gone on elsewhere.
    Elsewhere(Place),
}

/// What a stretch is there to show: the real code's decision the subtitle reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Moment {
    Plain,
    /// Reading on after a moment: the subtitle goes on saying what the pointer decided there for a while.
    On,
    Unsure,
    AyahEnd,
    Held,
    /// A breath inside an ayah, heard as silence by the voice's level.
    Quiet,
    Repeat,
    Skip,
    Back,
    Misheard,
    Away,
}

pub struct Stretch {
    /// Its first tick and the tick it ends at (`u32::MAX`: it goes on for good).
    from: u32,
    to: u32,
    /// Where the reader really is at its start and end (words into the passage), if on it.
    pub truth: Option<(f32, f32)>,
    pub ear: Ear,
    pub moment: Moment,
    /// What the reader does, for the subtitle.
    pub doing: &'static str,
}

/// Tick `k`'s time, on the microsecond so that ticks and stretches meet.
fn tick(k: u32) -> f64 {
    (f64::from(k) * TICK * 1e6).round() / 1e6
}

/// Ticks for at least `secs`.
fn ticks_for(secs: f32) -> u32 {
    (f64::from(secs) / TICK - 1e-6).ceil() as u32
}

impl Stretch {
    pub fn start(&self) -> f64 {
        tick(self.from)
    }

    pub fn until(&self) -> f64 {
        if self.to == u32::MAX { f64::INFINITY } else { tick(self.to) }
    }

    /// Where the reader really is at time `t` of it.
    fn truth_at(&self, t: f64) -> Option<f32> {
        let (a, b) = self.truth?;
        let share = ((t - self.start()) / (self.until() - self.start())).clamp(0.0, 1.0) as f32;
        Some(a + (b - a) * share)
    }
}

/// "ʾanʿama llāhu ʿalayhim" (4:69) for "ʾanʿamta ʿalayhim … ʿalayhim".
pub const MISHEARD: Place = Place { loc: (4, 69, 9), word: 11_147, from: 0.2, pace: 1.0, most: 1.5, conf: 0.9 };
/// Āyat al-Kursī, read on after al-Fātiḥa.
pub const ELSEWHERE: Place = Place { loc: (2, 255, 1), word: 5_173, from: 0.1, pace: PACE, most: 14.0, conf: 0.95 };

const READ: Ear = Ear::Reading { sure: true };
const PAUSE: Ear = Ear::Still { paused: true };
const HOLD: Ear = Ear::Still { paused: false };

/// Stretches laid back to back, so many ticks each.
#[derive(Default)]
struct Build(Vec<Stretch>);

impl Build {
    fn then(mut self, ticks: u32, truth: Option<(f32, f32)>, ear: Ear, moment: Moment, doing: &'static str) -> Self {
        let from = self.0.last().map_or(0, |s| s.to);
        self.0.push(Stretch { from, to: from.saturating_add(ticks), truth, ear, moment, doing });
        self
    }

    fn read(self, ticks: u32, (a, b): (f32, f32), moment: Moment, doing: &'static str) -> Self {
        self.then(ticks, Some((a, b)), READ, moment, doing)
    }

    fn still(self, ticks: u32, at: f32, ear: Ear, moment: Moment, doing: &'static str) -> Self {
        self.then(ticks, Some((at, at)), ear, moment, doing)
    }
}

/// A scene's recitation: its stretches back to back from 0. Slips start on a tick, so both listeners hear them
/// alike: the repeat 0.85 words behind the last word heard, the skip about 4 ahead of the pointer, the step back
/// over 4 behind the last word heard.
pub struct Script {
    pub scene: Scene,
    stretches: Vec<Stretch>,
    /// The listener plans (for tajweed timing) and sends the voice's level (for it, the hold or the dim), as the live
    /// one does when the settings ask.
    plans: bool,
    levels: bool,
}

/// A position in words into the passage as a plan has it: word `w`'s letters span `w` to `w + 0.95`.
fn planned(pos: f32) -> f32 {
    FIRST as f32 + pos.floor() + 0.95 * pos.fract()
}

/// Seconds of flow a plan runs on past its message (the listener's horizon).
const HORIZON: f32 = 0.44;

impl Script {
    /// `scene` as played with `voice`: a pause or a hold lasts as long as its setting needs to be seen.
    pub fn of(scene: Scene, voice: &Voice) -> Script {
        let m = &voice.motion;
        let (plans, levels) = (m.follow_tajweed, m.follow_tajweed || m.hold_when_quiet || m.quiet_dim < 100);
        let b = Build::default();
        let b = match scene {
            Scene::Read => b
                .then(2, None, Ear::Nothing, Moment::Plain, "is about to begin")
                .then(2, Some((0.0, 0.7)), Ear::Reading { sure: false }, Moment::Unsure, "begins 1:6")
                .read(6, (0.7, 2.95), Moment::Plain, "reads 1:6")
                .still(2, 2.95, PAUSE, Moment::Plain, "breathes at the ayah's end")
                .read(25, (3.0, 11.95), Moment::Plain, "reads 1:7")
                .still(2, 11.95, PAUSE, Moment::Plain, "stops at the ayah's end"),
            Scene::AyahEnd => {
                let delay = f32::from(m.ayah_fade_delay_ms) / 1000.0;
                let pause = if m.fade_at_ayah_end { ticks_for(delay + SEEN).max(6) } else { 8 };
                b.read(6, (0.6, 2.55), Moment::Plain, "reads 1:6")
                    .read(3, (2.55, 2.95), Moment::Plain, "draws out the ayah's last word")
                    .still(pause, 2.95, PAUSE, Moment::AyahEnd, "pauses at the ayah's end")
                    .read(5, (3.0, 4.75), Moment::On, "goes on with 1:7")
            }
            Scene::Hold => {
                let b = b.read(5, (3.0, 4.75), Moment::Plain, "reads 1:7").still(8, 4.75, HOLD, Moment::Held, "holds a letter").read(6, (4.75, 6.7), Moment::On, "reads on");
                // Heard by the voice's level, a breath inside the ayah as long as the dim's delay needs to be seen.
                if levels {
                    let breath = ticks_for(f32::from(m.ayah_fade_delay_ms) / 1000.0 + SEEN).max(5);
                    b.still(breath, 6.7, PAUSE, Moment::Quiet, "breathes inside the ayah").read(4, (6.7, 8.0), Moment::On, "reads on")
                } else {
                    b
                }
            }
            Scene::Slips => {
                let hold = ticks_for(f32::from(m.wait_ms) / 1000.0 + SEEN).max(5);
                b.read(7, (3.0, 5.45), Moment::Plain, "reads 1:7")
                    .still(hold, 4.25, HOLD, Moment::Repeat, "repeats a word")
                    .read(5, (4.25, 6.0), Moment::On, "reads on")
                    .read(2, (9.9, 10.6), Moment::Skip, "skips 4 words")
                    .still(1, 10.6, PAUSE, Moment::On, "pauses")
                    .read(1, (6.15, 6.5), Moment::Back, "goes back 4.5 words")
                    .read(8, (6.5, 9.3), Moment::On, "reads on")
            }
            Scene::Misheard => b
                .read(9, (3.0, 6.1), Moment::Plain, "reads 1:7")
                .then(4, Some((6.1, 7.5)), Ear::Elsewhere(MISHEARD), Moment::Misheard, "is misheard as 4:69")
                .read(7, (7.5, 9.9), Moment::On, "reads on"),
            Scene::Far => b
                .read(9, (8.3, 11.4), Moment::Plain, "reads 1:7")
                .read(2, (11.4, 11.95), Moment::Plain, "draws out the ayah's last word")
                .still(3, 11.95, PAUSE, Moment::Plain, "pauses at the ayah's end")
                .then(u32::MAX, None, Ear::Elsewhere(ELSEWHERE), Moment::Away, "reads on at 2:255"),
        };
        Script { scene, stretches: b.0, plans, levels }
    }

    pub fn stretches(&self) -> &[Stretch] {
        &self.stretches
    }

    /// The stretch playing at time `t`, and its index.
    pub fn at(&self, t: f64) -> (usize, &Stretch) {
        let i = self.stretches.iter().position(|s| t < s.until()).unwrap_or(self.stretches.len() - 1);
        (i, &self.stretches[i])
    }

    /// Where the reader really is at time `t` (words into the passage), if on it.
    pub fn truth(&self, t: f64) -> Option<f32> {
        self.at(t).1.truth_at(t)
    }

    /// Whether the reader is reading on along the passage at time `t` (the caret moves).
    pub fn moving(&self, t: f64) -> bool {
        self.at(t).1.truth.is_some_and(|(a, b)| a != b)
    }

    /// How long it lasts (the far scene: for good, its player ends it).
    pub fn len(&self) -> f64 {
        self.stretches.last().map_or(0.0, Stretch::until)
    }

    /// When the reader goes on elsewhere for good, if they do.
    pub fn moved(&self) -> Option<f64> {
        self.stretches.last().filter(|s| s.moment == Moment::Away).map(Stretch::start)
    }

    /// Every message the listener sends, in order: a perfect listener's (`Even`) or the real one's. Both hear the
    /// same stretches; only reading and pausing are heard differently.
    pub fn updates(&self, cadence: Cadence) -> Vec<Update> {
        let real = cadence == Cadence::Heard;
        let letters = |g: &Glyph| if real { g.letters } else { FINE };
        // A far run is over once it has gone as far as it goes.
        let end = match self.stretches.last() {
            Some(s @ Stretch { ear: Ear::Elsewhere(place), .. }) if s.to == u32::MAX => s.start() + f64::from(place.most / place.pace) + TICK,
            _ => self.len(),
        };
        let mut out: Vec<Update> = Vec::new();
        // What was sent last: a listener sends only changes; and the plans' seq, which grows with the place heard.
        let mut sent: Option<(u32, u16, &str)> = None;
        let mut seq = (0u32, None);
        for k in 0..(end / TICK - 1e-6).ceil() as u32 {
            let t = tick(k);
            let i = self.stretches.iter().position(|s| k < s.to).unwrap_or(self.stretches.len() - 1);
            let s = &self.stretches[i];
            let (first, last) = (k == s.from, k + 1 == s.to);
            let lag = if real && !first && !last { LAG[k as usize % 20] } else { Some(0.0) };
            let conf = if real { CONF[k as usize % 20] } else { 1.0 };
            let at = |pos: Option<f32>| on_passage(pos.unwrap_or(0.0), letters);
            let message = match s.ear {
                Ear::Nothing => None,
                Ear::Reading { sure } => lag.map(|lag| (at(s.truth_at(t - lag)), if sure { "Locked" } else { "Tentative" }, conf)),
                // The reader's last letter first, as it stops (the real listener catching up on it).
                Ear::Still { paused: false } => Some((at(s.truth_at(s.until())), "Locked", conf)),
                Ear::Still { paused: true } if !real => Some((at(s.truth_at(s.until())), "Paused", conf)),
                // Noticed late (or at the pause's last tick), a letter back.
                Ear::Still { paused: true } if k >= s.from + PAUSE_LATE || last => Some((letter_back(at(s.truth_at(s.until())), letters), "Paused", conf)),
                Ear::Still { paused: true } => Some((at(s.truth_at(s.until())), "Locked", conf)),
                Ear::Elsewhere(place) => Some((off_passage(&place, place.from + (place.pace * (t - s.start()) as f32).min(place.most)), "Locked", place.conf)),
            };
            let Some(((loc, word, letter, n), state, conf)) = message else { continue };
            if sent == Some((word, letter, state)) {
                continue;
            }
            sent = Some((word, letter, state));
            let late = if real && !matches!(s.ear, Ear::Elsewhere(_)) { JITTER[k as usize % 8] } else { 0.0 };
            let plan = (self.plans && state == "Locked" && loc.0 == 1).then(|| {
                if seq.1 < Some((word, letter)) {
                    seq = (seq.0 + 1, Some((word, letter)));
                }
                self.plan(s, t, (word, letter, n), seq.0)
            });
            let heard = Heard { loc, word, letter, letters: n, state: state.into(), conf, speed: PACE * UNITS_PER_WORD, t: t as f32, slid: None, alts: Vec::new(), plan };
            out.push(Update { at: t + late, heard, stretch: i });
        }
        out
    }
}

impl Script {
    /// The plan a listener that plans sends with a Locked message heard at tick time `t` in stretch `s` (word, letter,
    /// letters): the reader where they really are, flowing on at the stretch's pace for the horizon; a letter held
    /// crept across in the hold's time, and waited at its end.
    fn plan(&self, s: &Stretch, t: f64, (word, letter, n): (u32, u16, u16), seq: u32) -> Plan {
        let ev_end = word as f32 + 0.95 * f32::from(letter + 1) / f32::from(n);
        let now = planned(s.truth_at(t).unwrap_or(0.0)).max(word as f32 + 0.95 * f32::from(letter) / f32::from(n));
        let (knots, hold) = match s.ear {
            Ear::Still { paused: false } => {
                let secs = (s.until() - t) as f32;
                (vec![(secs, (ev_end - now).max(0.0))], Some((0.0, secs)))
            }
            _ => {
                let pace = s.truth.map_or(0.0, |(a, b)| (b - a) / (s.until() - s.start()) as f32);
                (vec![(HORIZON, 0.95 * pace * HORIZON)], None)
            }
        };
        Plan { seq, age: 0.0, ev: (word, letter), ev_end, now, knots, hold }
    }

    /// What the voice's level says as the reader reads, holds and pauses (none without `levels`), as the link's
    /// presence would: present from a stretch heard sounding on (two frames in) and the silence over a blip later;
    /// absent once a carry after the sound stopped, and a silence.
    pub fn voices(&self) -> Vec<(f64, presence::Heard)> {
        if !self.levels {
            return Vec::new();
        }
        let heard = |present, quiet| presence::Heard { present, quiet };
        let mut out = vec![(0.0, heard(true, false))];
        let mut sounding = true;
        for s in &self.stretches {
            let now = matches!(s.ear, Ear::Reading { .. } | Ear::Still { paused: false } | Ear::Elsewhere(_));
            match (sounding, now) {
                (true, false) => out.push((s.start() + 0.2, heard(false, true))),
                (false, true) => out.extend([(s.start() + 0.08, heard(true, true)), (s.start() + 0.23, heard(true, false))]),
                _ => {}
            }
            sounding = now;
        }
        out
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Cadence {
    /// A perfect listener: every step alike, on every tick.
    Even,
    /// As quran-voice hears a reader.
    #[default]
    Heard,
}

/// Seconds the real listener's word on each tick lags the reader (`None`: no message), and how sure it is,
/// a pattern of 20 ticks taken from the step pattern of a replay: a word held, then 0.9 of one at once. Every
/// reading stretch but its first and last tick goes by it, so a long one plays all of it.
const LAG: [Option<f64>; 20] = [
    Some(0.0), Some(-0.16), Some(-0.05), Some(0.09), Some(0.05), None, None, Some(-0.05), Some(-0.08), Some(-0.05),
    Some(0.0), Some(0.19), Some(0.28), Some(0.48), Some(0.68), Some(0.41), Some(0.0), Some(0.15), Some(0.35), Some(-0.12),
];
const CONF: [f32; 20] = [0.82, 0.98, 1.0, 1.0, 0.96, 0.9, 0.9, 0.86, 0.9, 1.0, 1.0, 1.0, 0.97, 0.83, 0.96, 1.0, 1.0, 0.99, 0.92, 0.66];
/// How late a message arrives after its tick (the replays only log a smoothed latency).
const JITTER: [f64; 8] = [0.012, 0.0, 0.031, 0.007, 0.022, 0.035, 0.004, 0.017];
/// The real listener notices a pause this many ticks late, and reports it a letter back.
const PAUSE_LATE: u32 = 2;

/// A message of the scripted listener: when it arrives (time into its scene) and the stretch it hears.
#[derive(Debug, Clone, PartialEq)]
pub struct Update {
    pub at: f64,
    pub heard: Heard,
    pub stretch: usize,
}

/// Position `pos` of the passage as the listener names it: (place, index in the Quran, letter, letters).
fn on_passage(pos: f32, letters: impl Fn(&Glyph) -> u16) -> (Loc, u32, u16, u16) {
    let i = (pos.max(0.0) as usize).min(words().count() - 1);
    let g = words().nth(i).expect("a word of the passage");
    let n = letters(g);
    let letter = (((pos - i as f32).clamp(0.0, 1.0) * f32::from(n)) as u16).min(n - 1);
    (g.loc, FIRST + i as u32, letter, n)
}

/// The letter before (in the word before at its first).
fn letter_back((loc, word, letter, n): (Loc, u32, u16, u16), letters: impl Fn(&Glyph) -> u16) -> (Loc, u32, u16, u16) {
    match (letter, word.checked_sub(FIRST + 1)) {
        (0, Some(i)) => {
            let g = words().nth(i as usize).expect("a word of the passage");
            (g.loc, word - 1, letters(g) - 1, letters(g))
        }
        _ => (loc, word, letter.saturating_sub(1), n),
    }
}

/// Position `pos` of a place off the passage.
fn off_passage(place: &Place, pos: f32) -> (Loc, u32, u16, u16) {
    let on = pos.floor();
    let loc = (place.loc.0, place.loc.1, place.loc.2 + on as u16);
    (loc, place.word + on as u32, ((pos - on) * 4.0) as u16, 4)
}

/// The scene a change of the voice settings shows at work; `None` for the pointer's and the range's look and
/// glides, which show wherever it is.
pub fn scene_for(old: &Voice, new: &Voice) -> Option<Scene> {
    let (a, b) = (&old.motion, &new.motion);
    let tajweed = |m: &quran_core::settings::VoiceMotion| (m.follow_tajweed, m.stale_wait_ms, m.strict_release, m.retract, m.hold_when_quiet, m.quiet_dim);
    if a.max_ahead != b.max_ahead || tajweed(a) != tajweed(b) {
        Some(Scene::Hold)
    } else if (a.wait_ms, a.fade_back, a.far_jump) != (b.wait_ms, b.fade_back, b.far_jump) {
        Some(Scene::Slips)
    } else if (a.fade_at_ayah_end, a.ayah_fade_delay_ms) != (b.fade_at_ayah_end, b.ayah_fade_delay_ms) {
        Some(Scene::AyahEnd)
    } else if (old.far_wait_ms, old.auto_scroll) != (new.far_wait_ms, new.auto_scroll) {
        Some(Scene::Far)
    } else {
        None
    }
}

#[cfg(test)]
mod tests;
