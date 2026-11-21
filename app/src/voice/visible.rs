//! The words on screen as the listener hears of them, and whether the reader chose them. A page the reader
//! turned to is where quran-voice's tracker looks first (`"by":"reader"`); words the pointer's own scrolling or
//! navigation put on screen are not the reader's choice, and after a wrong lock they are the wrong page. Words
//! are the pointer's when its last move of the view put them there (a flag cleared on each send stayed set
//! through the sub-line tail of an auto-scroll and marked the reader's next page as the pointer's).

use super::link::Loc;

/// Seconds between two messages.
const EVERY: f64 = 1.0;
/// Seconds a place the pointer sent the reader to may take to show (loading it included).
const LANDING: f64 = 10.0;

#[derive(Debug, Default)]
pub struct Visible {
    /// The words last sent (first and last), and when.
    sent: Option<(Loc, Loc)>,
    at: f64,
    /// The words on screen after the pointer last moved the view, kept up while it moves it.
    pointer: Option<(Loc, Loc)>,
    /// The pointer scrolled in the frame before (this frame's words show it); since when it has been sending the
    /// reader to a place, and the place.
    scrolled: bool,
    going: Option<(f64, Loc)>,
}

impl Visible {
    /// This frame's words on screen: whether to send them now, and if so whether the reader chose them.
    pub fn frame(&mut self, words: (Loc, Loc), now: f64) -> Option<bool> {
        if std::mem::take(&mut self.scrolled) || self.going.is_some_and(|(t, _)| now - t < LANDING) {
            self.pointer = Some(words);
        }
        // Landed once the place is on screen (these words are still the pointer's), or given up on.
        if self.going.is_some_and(|(t, to)| now - t >= LANDING || (words.0..=words.1).contains(&to)) {
            self.going = None;
        }
        if self.sent == Some(words) || now - self.at <= EVERY {
            return None;
        }
        self.sent = Some(words);
        self.at = now;
        Some(self.pointer != Some(words))
    }

    /// The pointer scrolls the view this frame.
    pub fn scrolled(&mut self) {
        self.scrolled = true;
    }

    /// The pointer sends the reader to the place `to` this frame.
    pub fn going(&mut self, to: Loc, now: f64) {
        self.going = Some((now, to));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f64 = 1.0 / 60.0;

    /// Words `first..first + 14` of 2:7 as a range on screen.
    fn range(first: u16) -> (Loc, Loc) {
        ((2, 7, first), (2, 7, first + 14))
    }

    /// Runs frames from `t` to `until` with the words of `at` (the time), the pointer scrolling while `scroll` says
    /// so; returns the messages sent (time, first word, by the reader) and the time reached.
    fn run(v: &mut Visible, mut t: f64, until: f64, at: impl Fn(f64) -> u16, scroll: impl Fn(f64) -> bool) -> (Vec<(f64, u16, bool)>, f64) {
        let mut sent = Vec::new();
        while t < until {
            let words = range(at(t));
            if let Some(by_reader) = v.frame(words, t) {
                sent.push((t, words.0.2, by_reader));
            }
            if scroll(t) {
                v.scrolled();
            }
            t += DT;
        }
        (sent, t)
    }

    #[test]
    fn the_page_the_reader_has_open_is_theirs() {
        let mut v = Visible::default();
        let (sent, _) = run(&mut v, 0.0, 2.0, |_| 1, |_| false);
        assert_eq!(sent.iter().map(|s| (s.1, s.2)).collect::<Vec<_>>(), [(1, true)]);
    }

    #[test]
    fn an_auto_scroll_is_the_pointers_to_its_last_sub_line_frame_and_the_reader_s_next_page_theirs() {
        // The pointer scrolls for 1.6 s, the words moving on a line (here 3 words) every 0.2 s and still in the
        // last frames; the reader then scrolls a page on by hand.
        let mut v = Visible::default();
        let (mut sent, t) = run(&mut v, 0.0, 1.2, |_| 1, |_| false);
        let scrolling = |t: f64| (1.2..2.8).contains(&t);
        let line = |t: f64| 1 + 3 * ((t.clamp(1.2, 2.8) - 1.2) / 0.2).ceil() as u16;
        let (more, t) = run(&mut v, t, 6.0, line, scrolling);
        sent.extend(more);
        let (more, _) = run(&mut v, t, 8.0, |_| 60, |_| false);
        sent.extend(more);
        let first = sent.iter().position(|s| s.1 != 1).unwrap();
        assert!(sent[first..].iter().all(|s| (s.1 == 60) == s.2), "{sent:?}");
        assert_eq!(sent.last().map(|s| (s.1, s.2)), Some((60, true)), "{sent:?}");
    }

    #[test]
    fn a_place_the_pointer_went_to_is_its_own_while_it_loads_and_after() {
        // The pointer sends the reader to 2:7:235, not loaded yet: the reader's page stays for 1 s, the surah's top
        // shows for 1.5 s, then the place; later the reader goes back to their own page.
        let mut v = Visible::default();
        let (mut sent, t) = run(&mut v, 0.0, 1.5, |_| 1, |_| false);
        v.going((2, 7, 235), t);
        let (more, t) = run(&mut v, t, 4.0, |t| if t < 2.5 { 1 } else { 200 }, |_| false);
        sent.extend(more);
        let (more, t) = run(&mut v, t, 7.0, |_| 230, |_| false);
        sent.extend(more);
        let (more, _) = run(&mut v, t, 9.0, |_| 1, |_| false);
        sent.extend(more);
        assert_eq!(sent.iter().map(|s| (s.1, s.2)).collect::<Vec<_>>(), [(1, true), (200, false), (230, false), (1, true)]);
    }

    #[test]
    fn a_place_that_never_shows_stops_being_the_pointers() {
        let mut v = Visible::default();
        v.going((2, 7, 500), 0.0);
        let (sent, _) = run(&mut v, 0.0, LANDING + 2.0, |t| if t < LANDING { 1 } else { 40 }, |_| false);
        assert_eq!(sent.iter().map(|s| (s.1, s.2)).collect::<Vec<_>>(), [(1, false), (40, true)]);
    }
}
