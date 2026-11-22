//! A jump ahead held until the reading goes on there (tuned movement). A misreading the listener follows for a moment,
//! a few lines ahead, comes back when the reader corrects themselves at the mistaken word: followed, the pointer went
//! there and back. In the user's recitations such excursions came back within 0.3-2.2 s, with at most one report
//! further on there; a real skip went on with 4-5 reports. So a jump of more than a few words, within about a page,
//! waits for two more reports going on from it, or one and 1.5 s; a report back near the pointer drops it. Steps back
//! (corrections) and the reading's own flow are never held; places further off are `follow`'s.

/// Words ahead of the pointer's place beyond which a report is a jump.
const JUMP: f32 = 3.0;
/// Words ahead up to which a jump is held (about a page; further off is a far place).
pub const PAGE: f32 = 150.0;
/// Reports going on from a held jump, or one and this long, that confirm it.
const CONFIRM: u8 = 2;
const CONFIRM_SECS: f64 = 1.5;
/// Words a report may go on from the held place and still continue it.
const ON: f32 = 4.0;

#[derive(Debug, Default)]
pub struct Hold {
    /// The jump held: where it was first heard, its latest place, when it was first heard, and the reports since that
    /// went on from it.
    held: Option<(f32, f32, f64, u8)>,
    /// Where and when the jump the last report confirmed was first heard.
    confirmed: Option<(f32, f64)>,
}

impl Hold {
    /// Whether to follow a report of position `c` at clock time `now`, the pointer going to `at` (`None`: it does not
    /// show, and nothing is held).
    pub fn take(&mut self, c: f32, at: Option<f32>, now: f64) -> bool {
        self.confirmed = None;
        if !at.is_some_and(|at| c > at + JUMP && c <= at + PAGE) {
            self.held = None;
            return true;
        }
        match self.held {
            Some((first, was, since, n)) if c >= was - 0.5 && c <= was + ON => {
                let n = if c > was + 0.25 { n + 1 } else { n };
                if n >= CONFIRM || (n > 0 && now - since >= CONFIRM_SECS) {
                    self.held = None;
                    self.confirmed = Some((first, since));
                    return true;
                }
                self.held = Some((first, c.max(was), since, n));
            }
            _ => self.held = Some((c, c, now, 0)),
        }
        false
    }

    /// A jump is held (the listener's place is there, not the pointer's).
    pub fn holding(&self) -> bool {
        self.held.is_some()
    }

    /// Where and when the jump the last report taken confirmed was first heard (the reading arrived there then), once.
    pub fn confirmed(&mut self) -> Option<(f32, f64)> {
        self.confirmed.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reports every 0.32 s from `t` at `places`, the pointer going to `at` (followed reports move it); which were
    /// followed.
    fn reports(hold: &mut Hold, at: &mut f32, places: &[f32], t: f64) -> Vec<bool> {
        (0..)
            .zip(places)
            .map(|(k, &c)| {
                let take = hold.take(c, Some(*at), t + 0.32 * f64::from(k));
                if take {
                    *at = c;
                }
                take
            })
            .collect()
    }

    #[test]
    fn reading_on_and_stepping_back_are_never_held() {
        let (mut hold, mut at) = (Hold::default(), 100.2);
        let flow = [100.6, 101.2, 101.7, 102.4, 103.1, 105.0, 104.2, 99.0, 99.6];
        assert!(reports(&mut hold, &mut at, &flow, 0.0).iter().all(|t| *t), "a word or two at a time, and back");
        // A far place is not this hold's: followed, as `follow` decides.
        assert!(hold.take(9_000.0, Some(at), 4.0));
        assert!(hold.take(0.5, None, 5.0), "nothing shown: nothing to hold");
    }

    #[test]
    fn a_misreading_two_lines_ahead_that_comes_back_never_moves_the_pointer() {
        // At 100, heard 17 words on (two lines) for 0.64 s, once going on there, then back at the mistaken word.
        let (mut hold, mut at) = (Hold::default(), 100.4);
        let followed = reports(&mut hold, &mut at, &[117.3, 117.9, 100.6, 101.1], 0.0);
        assert_eq!(followed, [false, false, true, true]);
        assert!(at < 102.0, "{at}");
        // Held for a long while on the one word ahead: still not followed (2.2 s, as one in the logs).
        let (mut hold, mut at) = (Hold::default(), 50.2);
        assert!(reports(&mut hold, &mut at, &[56.5; 7], 0.0).iter().all(|t| !t));
    }

    #[test]
    fn a_real_skip_that_reads_on_there_is_followed_within_the_hold() {
        let (mut hold, mut at) = (Hold::default(), 200.3);
        let followed = reports(&mut hold, &mut at, &[223.4, 224.0, 224.6, 225.3], 0.0);
        assert_eq!(followed, [false, false, true, true], "two reports going on: 0.64 s");
        // The reading arrived where the jump was first heard, and when (said once, for the report that confirmed it).
        let (mut hold, mut at) = (Hold::default(), 200.3);
        reports(&mut hold, &mut at, &[223.4, 224.0], 5.0);
        assert!(hold.take(224.6, Some(at), 5.64) && hold.confirmed() == Some((223.4, 5.0)));
        assert!(hold.take(225.3, Some(224.6), 5.96) && hold.confirmed().is_none());
        // One report on, then a long word: followed once 1.5 s have passed.
        let (mut hold, mut at) = (Hold::default(), 300.1);
        let followed = reports(&mut hold, &mut at, &[308.2, 308.8, 308.8, 308.8, 308.8, 308.8], 0.0);
        assert_eq!(followed, [false, false, false, false, false, true]);
    }
}
