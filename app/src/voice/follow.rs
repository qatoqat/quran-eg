//! Whether to take the reader to a place heard off screen. A place near where the recitation was being
//! followed (the next page, the next surah, a few lines back), or any place before the recitation has
//! been followed well, is followed after a second. A far one (another surah, more than about a page
//! away) is taken first for a slip or the listener's mistake: the pointer hides and the page stays, and
//! the reader goes there only once the recitation has kept being followed there, reading on, for the
//! set time. Places are compared by their index in the whole Quran, so surah borders need no special
//! case.

use super::link::Loc;

/// Seconds surely locked on screen before that place is where the recitation is being followed.
const HOME_AFTER: f64 = 2.0;
/// Seconds any place must be off screen before the reader goes there.
const AWAY: f64 = 1.0;
/// Words behind and ahead of the place being followed that are near it: about a mushaf page (80
/// back keeps al-Ikhlas near the end of an-Nas).
const BACK: u32 = 80;
const AHEAD: u32 = 150;
/// Words back and on from the last place heard there that still continue a far place's run.
const RUN: (u32, u32) = (3, 30);
/// Words a far place's run must have advanced before the reader is taken there.
const ADVANCE: u32 = 3;
/// Seconds a far place may go unheard and still continue its run.
const STALE: f64 = 3.0;

/// Where the recitation was being followed on screen.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Home {
    word: u32,
    loc: Loc,
}

/// A far place heard while the recitation was being followed elsewhere: a slip or a mishearing, until
/// it has been followed there long enough.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stray {
    /// Where the recitation was being followed, and where it was heard instead (lately).
    pub from: Loc,
    pub to: Loc,
    /// When it was first heard there.
    pub since: f64,
    /// The index of the run's first and latest word, and when that was heard.
    first: u32,
    word: u32,
    heard: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Not yet (off screen for less than a second, or unsure).
    Wait,
    /// Take the reader there.
    Go,
    /// A far place not followed long enough: lost for now.
    Stray,
}

#[derive(Debug, Default)]
pub struct Pages {
    /// Since when the place heard has been surely locked on screen.
    streak: Option<f64>,
    home: Option<Home>,
    stray: Option<Stray>,
    /// Since when the place heard has been off screen.
    away_since: Option<f64>,
}

impl Pages {
    /// The place heard (`word`, the index in the whole Quran, at `loc`) is on screen; `sure`: locked
    /// and confident. Returns the far place the reader had strayed to, once they are surely back.
    pub fn on_screen(&mut self, word: u32, loc: Loc, sure: bool, now: f64) -> Option<Stray> {
        self.away_since = None;
        if !sure {
            return None;
        }
        if now - *self.streak.get_or_insert(now) >= HOME_AFTER {
            self.home = Some(Home { word, loc });
        }
        self.stray.take()
    }

    /// The listener is looking for the place, or unsure of it.
    pub fn lost(&mut self) {
        self.streak = None;
    }

    /// The place heard is not on screen. `wait`: seconds a far place must keep being followed before
    /// the reader goes there.
    pub fn off_screen(&mut self, word: u32, loc: Loc, sure: bool, now: f64, wait: f64) -> Verdict {
        self.streak = None;
        let away = now - *self.away_since.get_or_insert(now);
        if !sure {
            return if self.stray.is_some() { Verdict::Stray } else { Verdict::Wait };
        }
        let far = self.home.filter(|home| wait > 0.0 && (word + BACK < home.word || word > home.word + AHEAD));
        let Some(home) = far else {
            self.stray = None;
            return self.go_if(away >= AWAY);
        };
        let s = match self.stray {
            Some(s) if word + RUN.0 >= s.word && word <= s.word + RUN.1 && now - s.heard <= STALE => s,
            // Somewhere else: a new run, and the clock starts again.
            _ => Stray { from: home.loc, to: loc, since: now, first: word, word, heard: now },
        };
        self.stray = Some(Stray { to: loc, word, heard: now, ..s });
        if now - s.since >= wait.max(AWAY) && word >= s.first + ADVANCE {
            // The reader really went on there.
            self.home = None;
            self.stray = None;
            return self.go_if(true);
        }
        Verdict::Stray
    }

    fn go_if(&mut self, go: bool) -> Verdict {
        if !go {
            return Verdict::Wait;
        }
        self.away_since = None;
        Verdict::Go
    }

    /// Seconds until a place off screen has been away long enough to be followed.
    pub fn wake(&self, now: f64) -> Option<f32> {
        self.away_since.map(|since| (since + AWAY - now) as f32).filter(|left| *left > 0.0)
    }

    /// The far place heard while the recitation is lost for now.
    pub fn stray(&self) -> Option<&Stray> {
        self.stray.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOME: (u32, Loc) = (10_000, (20, 50, 3));

    /// Surely locked on screen at the home place for 2.5 s (updates every 0.32 s), from `t`.
    fn followed(t: f64) -> (Pages, f64) {
        let mut pages = Pages::default();
        let mut t = t;
        for _ in 0..8 {
            assert_eq!(pages.on_screen(HOME.0, HOME.1, true, t), None);
            t += 0.32;
        }
        (pages, t)
    }

    /// Updates off screen every 0.32 s from `t` for `secs`, `step` words on each; the verdicts.
    fn heard(pages: &mut Pages, word: u32, step: u32, sure: bool, t: f64, secs: f64) -> Vec<(f64, Verdict)> {
        let mut out = Vec::new();
        let mut k = 0;
        while f64::from(k) * 0.32 <= secs {
            let now = t + f64::from(k) * 0.32;
            out.push((now, pages.off_screen(word + k * step, (30, 12, 1), sure, now, 5.0)));
            k += 1;
        }
        out
    }

    #[test]
    fn with_nothing_followed_yet_any_place_goes_after_a_second() {
        let mut pages = Pages::default();
        assert_eq!(pages.off_screen(50_000, (60, 1, 1), true, 0.0, 5.0), Verdict::Wait);
        assert_eq!(pages.wake(0.4), Some(0.6));
        assert_eq!(pages.off_screen(50_000, (60, 1, 1), true, 1.01, 5.0), Verdict::Go);
        // Less than 2 s followed on screen is not yet a home.
        let mut pages = Pages::default();
        pages.on_screen(HOME.0, HOME.1, true, 0.0);
        pages.on_screen(HOME.0, HOME.1, true, 1.5);
        pages.off_screen(50_000, (60, 1, 1), true, 1.8, 5.0);
        assert_eq!(pages.off_screen(50_000, (60, 1, 1), true, 2.9, 5.0), Verdict::Go);
    }

    #[test]
    fn a_far_place_is_lost_until_it_has_been_followed_there_reading_on() {
        let (mut pages, t) = followed(0.0);
        // Another surah, 5000 words away, read on at a word a step: lost at 1 s and 3 s, gone at 5 s.
        let verdicts = heard(&mut pages, HOME.0 + 5000, 1, true, t, 6.0);
        for (now, verdict) in &verdicts {
            let want = if now - t >= 5.0 { Verdict::Go } else { Verdict::Stray };
            assert_eq!(*verdict, want, "at {:.2} s", now - t);
            if *verdict == Verdict::Go {
                break;
            }
        }
        assert!(verdicts.iter().any(|(_, v)| *v == Verdict::Go));
        assert!(pages.stray().is_none(), "nothing pending once there");
    }

    #[test]
    fn a_far_place_held_unsure_or_hopping_about_never_confirms() {
        // Held on one word.
        let (mut pages, t) = followed(0.0);
        assert!(heard(&mut pages, 40_000, 0, true, t, 8.0).iter().all(|(_, v)| *v == Verdict::Stray));
        // Unsure (paused, drifting, low confidence): lost, never taken there.
        let (mut pages, t) = followed(0.0);
        pages.off_screen(40_000, (30, 12, 1), true, t, 5.0);
        assert!(heard(&mut pages, 40_001, 1, false, t + 0.32, 8.0).iter().all(|(_, v)| *v == Verdict::Stray));
        // Hopping from one far place to another: the clock starts again each time.
        let (mut pages, t) = followed(0.0);
        for k in 0..30 {
            let now = t + f64::from(k) * 0.32;
            let word = if k % 2 == 0 { 40_000 + k } else { 60_000 + k };
            assert_eq!(pages.off_screen(word, (30, 12, 1), true, now, 5.0), Verdict::Stray, "{k}");
        }
        // A run not heard for over 3 s is over: the next place there starts a new one.
        let (mut pages, t) = followed(0.0);
        heard(&mut pages, 40_000, 1, true, t, 3.0);
        let late = t + 3.0 + STALE + 0.5;
        assert_eq!(pages.off_screen(40_012, (30, 12, 1), true, late, 5.0), Verdict::Stray);
        assert_eq!(pages.stray().unwrap().since, late);
    }

    #[test]
    fn back_on_screen_the_slip_is_reported_once() {
        let (mut pages, t) = followed(0.0);
        heard(&mut pages, 40_000, 1, true, t, 3.0);
        let stray = pages.stray().copied().unwrap();
        assert_eq!((stray.from, stray.since), (HOME.1, t));
        assert_eq!(pages.on_screen(HOME.0 + 1, HOME.1, false, t + 3.3), None, "not sure yet");
        assert_eq!(pages.on_screen(HOME.0 + 1, HOME.1, true, t + 3.6), Some(stray));
        assert_eq!(pages.on_screen(HOME.0 + 2, HOME.1, true, t + 3.9), None);
        assert!(pages.stray().is_none());
    }

    #[test]
    fn reading_on_or_stepping_back_near_is_followed_after_a_second() {
        for (word, why) in [(HOME.0 + 40, "the next page"), (HOME.0 + 1, "the next surah"), (HOME.0 - 30, "a few lines back")] {
            let (mut pages, t) = followed(0.0);
            assert_eq!(pages.off_screen(word, (21, 1, 1), true, t, 5.0), Verdict::Wait, "{why}");
            assert_eq!(pages.off_screen(word, (21, 1, 1), true, t + 1.1, 5.0), Verdict::Go, "{why}");
        }
        // Without a wait every place is as near.
        let (mut pages, t) = followed(0.0);
        pages.off_screen(60_000, (60, 1, 1), true, t, 0.0);
        assert_eq!(pages.off_screen(60_000, (60, 1, 1), true, t + 1.1, 0.0), Verdict::Go);
        // al-Ikhlas (112:1:1, word 77371) from the end of an-Nas (114:6:3, word 77428).
        let mut pages = Pages::default();
        for k in 0..8 {
            pages.on_screen(77_428, (114, 6, 3), true, f64::from(k) * 0.32);
        }
        pages.off_screen(77_371, (112, 1, 1), true, 3.0, 5.0);
        assert_eq!(pages.off_screen(77_371, (112, 1, 1), true, 4.1, 5.0), Verdict::Go);
    }
}
