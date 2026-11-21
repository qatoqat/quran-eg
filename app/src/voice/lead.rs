//! The basmala that opens the next surah. The listener hears it as a formula (0:2:w), not as a place; right after the
//! last ayah of surah S it is S+1's opening (every surah but at-Tawbah has one), so it is taken as a place on S+1's
//! Bismillah line, which the reader lays out as `(S+1, 0, w)` just before S+1's first word: the pointer shows there,
//! and the reader goes there at once, as it is where the recitation goes on (no wait as for a far place). A basmala no
//! place leads into is read on a Bismillah line on screen, if there is one: the one the mouse pointer is on or near,
//! else the first after the place last followed (a page can hold three), else the first; wherever the reader goes on,
//! the pointer follows. The takbir said between the last surahs (or its tahlil or tahmid form) after a surah's last
//! ayah takes the reader to the next surah's head too, and after an-Nas to al-Fatihah, whose basmala is its first ayah.

use egui::{Pos2, Rect};
use quran_core::mushaf_layout::has_bismillah_line;
use quran_core::nav::CHAPTER_COUNT;
use quran_core::reference::{verses_in, words_in};

use super::link::{Heard, Loc};

/// The listener's takbir forms (surah 0): the takbir, the tahlil and the tahmid.
const TAKBIR: u16 = 5;
const TAHMID: u16 = 7;
use super::marker;
use crate::reader::Slot;

/// How long after the last ayah the basmala still leads on: a breath, a sip of water. A reader back after longer may
/// have chosen another place.
const WITHIN: f64 = 30.0;

#[derive(Debug, Default)]
pub struct Lead {
    /// The last place followed (locked, drifting or paused) outside the formulas, and when.
    followed: Option<(Loc, u32, f64)>,
    /// The surah a basmala now leads into, and the index of its first word in the whole Quran.
    into: Option<(u16, u32)>,
    /// The Bismillah lines on screen (their surahs, top to bottom, and each one's middle), the mouse pointer's height
    /// over the page if it is there, and a line's height.
    shown: Vec<(u16, f32)>,
    cursor: Option<f32>,
    line: f32,
    /// The surah a takbir heard after a surah's last ayah leads into, until the marker takes it.
    takbir: Option<u16>,
}

impl Lead {
    /// The view: `words` on screen in `area`, the mouse pointer at `cursor`.
    pub fn view(&mut self, words: &[Slot], area: Rect, cursor: Option<Pos2>) {
        self.shown.clear();
        let firsts = words.iter().filter(|w| w.0.1 == 0 && w.0.2 == 1 && area.intersects(w.1));
        self.shown.extend(firsts.map(|w| (w.0.0, w.1.center().y)));
        self.cursor = cursor.filter(|p| area.contains(*p)).map(|p| p.y);
        self.line = words.first().map_or(40.0, |w| w.1.height());
    }

    /// The Bismillah line on screen a basmala no place leads into is read on: the one the mouse pointer is on or near,
    /// else the first after the place last followed, else the first.
    fn on_screen(&self) -> Option<u16> {
        let distance = |s: &&(u16, f32), y: f32| (s.1 - y).abs();
        let near = self.cursor.and_then(|y| self.shown.iter().min_by(|a, b| distance(a, y).total_cmp(&distance(b, y))).filter(|s| distance(s, y) < 2.0 * self.line));
        let after = self.followed.and_then(|(loc, ..)| self.shown.iter().find(|s| s.0 > loc.0));
        near.or(after).or(self.shown.first()).map(|s| s.0)
    }

    /// Takes in a message at clock time `now`. A basmala leading into the next surah comes back as a place on its
    /// Bismillah line, the words' indices just before the surah's first (a plan, in the listener's own indices, is
    /// dropped); anything else comes back as it is. The second value is true for the basmala's first message.
    pub fn heard(&mut self, h: Heard, now: f64) -> (Heard, bool) {
        let (s, a, w) = h.loc;
        if s == 0 && (TAKBIR..=TAHMID).contains(&a) && h.state == "Locked" {
            let next = self.followed.filter(|f| now - f.2 <= WITHIN).and_then(|(loc, word, _)| next_surah(loc, word));
            self.takbir = next.map(|n| n.0).or(self.takbir);
        }
        if (s, a) != (0, 2) {
            self.into = None;
            if s > 0 && matches!(h.state.as_str(), "Locked" | "Drifting" | "Paused") {
                self.followed = Some((h.loc, h.word, now));
            }
            return (h, false);
        }
        let begins = self.into.is_none() && h.state == "Locked";
        if begins {
            let next = self.followed.filter(|f| now - f.2 <= WITHIN).and_then(|(loc, word, _)| next_surah(loc, word));
            self.into = next.or_else(|| self.on_screen().and_then(|s| Some((s, first_word(s)?))));
        }
        // Al-Fatihah's basmala is its first ayah; any other surah's is its Bismillah line, the four words before its first.
        let at = |(surah, first): (u16, u32)| match surah {
            1 => ((1, 1, w), u32::from(w) - 1),
            _ => ((surah, 0, w), first + u32::from(w) - 5),
        };
        match self.into.filter(|_| (1..=4).contains(&w)).map(at) {
            Some((loc, word)) => (Heard { loc, word, plan: None, slid: None, alts: Vec::new(), ..h }, begins),
            None => (h, false),
        }
    }

    /// The surah a takbir after a surah's last ayah leads into, once.
    pub fn takbir(&mut self) -> Option<u16> {
        self.takbir.take()
    }
}

/// The point at position `c` of the place heard last (`anchor`), as `marker::point` finds it; a basmala leading into its
/// surah whose Bismillah line is not laid out (a flowing verse, say) is held at the start of the surah's first word.
pub fn point(c: f32, anchor: &Heard, words: &[Slot], lift: f32) -> Option<(egui::Pos2, usize)> {
    let (s, a, w) = anchor.loc;
    if s == 0 || a > 0 || words.binary_search_by(|(l, ..)| l.cmp(&anchor.loc)).is_ok() {
        return marker::point(c, anchor, words, lift);
    }
    let first = anchor.word + 5 - u32::from(w);
    marker::point(c.max(first as f32), &Heard { loc: (s, 1, 1), word: first, ..anchor.clone() }, words, lift)
}

/// The index of surah `s`'s first word in the whole Quran, as the listener counts (al-Fatihah's first is 0).
fn first_word(s: u16) -> Option<u32> {
    let mut n = 0;
    for c in 1..s {
        for a in 1..=verses_in(c)? {
            n += words_in(c, a)?;
        }
    }
    u32::try_from(n).ok()
}

/// The surah a basmala heard after `loc` (word `word` of the whole Quran) opens, with the index of its first word: the
/// next one after a surah's last ayah, or the one whose first word the listener locked on just before (it guesses at
/// the next surah as the basmala begins); if that surah opens with a basmala.
fn next_surah((s, a, w): Loc, word: u32) -> Option<(u16, u32)> {
    if (a, w) == (1, 1) && has_bismillah_line(s) {
        return Some((s, word));
    }
    if s == CHAPTER_COUNT && a == verses_in(s)? {
        // After an-Nas, the Quran begins again.
        return Some((1, 0));
    }
    let next = s + 1;
    if a != verses_in(s)? || next > CHAPTER_COUNT || !has_bismillah_line(next) {
        return None;
    }
    let left = u32::try_from(words_in(s, a)?.checked_sub(usize::from(w))?).ok()?;
    Some((next, word + left + 1))
}

#[cfg(test)]
mod tests {
    use egui::{LayerId, Rect, pos2, vec2};
    use quran_core::settings::Voice as Settings;

    use super::super::marker::{Marker, Shown, Stage};
    use super::super::motion::Params;
    use super::*;

    fn heard(loc: Loc, word: u32, state: &str) -> Heard {
        Heard { loc, word, letter: 0, letters: 4, state: state.into(), conf: 1.0, speed: 0.0, t: 0.0, slid: None, alts: Vec::new(), plan: None }
    }

    /// The basmala as the listener sends it: 0:2:w with al-Fatiha's indices.
    fn basmala(w: u16, state: &str) -> Heard {
        heard((0, 2, w), u32::from(w) - 1, state)
    }

    #[test]
    fn the_basmala_after_a_surahs_last_ayah_is_the_next_surahs() {
        // At-Tahrim ends at 66:12:17 (word 72459 of the whole Quran); al-Mulk's first word is 72460. As heard live: a
        // tentative guess at 67:1:1 and the basmala searched for come between, and change nothing.
        let mut lead = Lead::default();
        lead.heard(heard((66, 12, 17), 72_459, "Locked"), 10.0);
        lead.heard(heard((67, 1, 1), 72_460, "Tentative"), 10.5);
        assert_eq!(lead.heard(basmala(2, "Searching"), 11.0).0.loc, (0, 2, 2), "not before it is locked");
        let (h, first) = lead.heard(basmala(3, "Locked"), 11.5);
        assert_eq!((h.loc, h.word, first), ((67, 0, 3), 72_458, true));
        let (h, first) = lead.heard(basmala(4, "Paused"), 12.0);
        assert_eq!((h.loc, h.word, first), ((67, 0, 4), 72_459, false), "the basmala goes on there, whatever its state");
        // The surah's first word follows on from the basmala's last.
        assert_eq!(lead.heard(heard((67, 1, 1), 72_460, "Locked"), 13.0).0.loc, (67, 1, 1));
        // As heard live after al-Qalam's last ayah: al-Haaqqah's first word locked as the basmala begins.
        let mut lead = Lead::default();
        lead.heard(heard((68, 52, 5), 73_092, "Locked"), 0.0);
        lead.heard(heard((69, 1, 1), 73_093, "Locked"), 1.0);
        assert_eq!(lead.heard(basmala(2, "Locked"), 1.3).0, Heard { loc: (69, 0, 2), word: 73_090, ..basmala(2, "Locked") });
    }

    #[test]
    fn a_basmala_anywhere_else_stays_a_formula() {
        let cases = [
            // Mid-surah, as when starting over: 65:1:2.
            (heard((65, 1, 2), 70_000, "Locked"), 0.0),
            // Al-Fatihah opens with its own basmala, its first ayah.
            (heard((1, 1, 1), 0, "Locked"), 0.0),
            // Al-Anfal's last ayah: at-Tawbah has no basmala.
            (heard((8, 75, 21), 13_000, "Locked"), 0.0),
            // An-Naml 27:30, the basmala inside an ayah: not a last ayah.
            (heard((27, 30, 4), 31_000, "Locked"), 0.0),
            // At-Tahrim's end, but long ago.
            (heard((66, 12, 17), 72_459, "Locked"), -60.0),
            // At-Tahrim's end, only guessed.
            (heard((66, 12, 17), 72_459, "Tentative"), 0.0),
        ];
        for (before, at) in cases {
            let mut lead = Lead::default();
            lead.heard(before.clone(), at);
            assert_eq!(lead.heard(basmala(1, "Locked"), 1.0).0.loc, (0, 2, 1), "after {:?}", before.loc);
        }
        // Nothing followed yet.
        assert_eq!(Lead::default().heard(basmala(1, "Locked"), 1.0).0.loc, (0, 2, 1));
    }

    #[test]
    fn a_basmala_no_place_leads_into_is_read_on_a_bismillah_line_on_screen() {
        assert_eq!((first_word(1), first_word(67), first_word(69)), (Some(0), Some(72_460), Some(73_093)));
        // Page 604: al-Ikhlas's ayahs, then al-Falaq's and an-Nas's Bismillah lines (middles at 120 and 320).
        let line = |y: f32, l: Loc| (l, Rect::from_min_size(pos2(100.0, y - 20.0), vec2(50.0, 40.0)), y + 8.0, 20.0);
        let words = [line(40.0, (112, 4, 1)), line(120.0, (113, 0, 1)), line(120.0, (113, 0, 2)), line(320.0, (114, 0, 1))];
        let area = Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 400.0));
        let at = |lead: &mut Lead, now: f64| lead.heard(basmala(1, "Locked"), now).0.loc;
        // Nothing followed: the first on screen.
        let mut lead = Lead::default();
        lead.view(&words, area, None);
        assert_eq!(at(&mut lead, 1.0), (113, 0, 1));
        // An-Nas's first ayah followed long ago: the first after it (none), so the first; al-Ikhlas's: al-Falaq's.
        let mut lead = Lead::default();
        lead.heard(heard((114, 1, 1), 77_400, "Locked"), 0.0);
        lead.view(&words, area, None);
        assert_eq!(at(&mut lead, 100.0), (113, 0, 1));
        let mut lead = Lead::default();
        lead.heard(heard((112, 2, 1), 77_380, "Locked"), 0.0);
        lead.view(&words, area, None);
        assert_eq!(at(&mut lead, 100.0), (113, 0, 1));
        // The mouse pointer near an-Nas's line wins; far from both lines, it is not a hint.
        let mut lead = Lead::default();
        lead.view(&words, area, Some(pos2(200.0, 300.0)));
        assert_eq!(at(&mut lead, 1.0), (114, 0, 1));
        let mut lead = Lead::default();
        lead.view(&words, area, Some(pos2(200.0, 220.0)));
        assert_eq!(at(&mut lead, 1.0), (113, 0, 1));
        // The place last followed leading into the next surah still wins, on screen or not.
        let mut lead = Lead::default();
        lead.heard(heard((66, 12, 17), 72_459, "Locked"), 0.0);
        lead.view(&words, area, Some(pos2(200.0, 300.0)));
        assert_eq!(at(&mut lead, 1.0), (67, 0, 1));
    }

    #[test]
    fn a_takbir_after_a_surah_leads_into_the_next_and_after_an_nas_into_al_fatihah() {
        let takbir = |a: u16| heard((0, a, 1), 0, "Locked");
        // Al-Falaq's last ayah, then the takbir: an-Nas; the basmala after it is an-Nas's Bismillah line.
        let mut lead = Lead::default();
        let last = words_in(113, 5).unwrap() as u16;
        lead.heard(heard((113, 5, last), 77_410, "Locked"), 0.0);
        lead.heard(takbir(5), 1.0);
        assert_eq!(lead.takbir(), Some(114));
        assert_eq!(lead.takbir(), None, "once");
        assert_eq!(lead.heard(basmala(1, "Locked"), 2.0).0.loc, (114, 0, 1));
        // An-Nas's last ayah, the tahlil: al-Fatihah; its basmala is al-Fatihah's first ayah, words 0-3.
        let mut lead = Lead::default();
        lead.heard(heard((114, 6, 3), 77_428, "Locked"), 0.0);
        lead.heard(takbir(6), 1.0);
        assert_eq!(lead.takbir(), Some(1));
        let (h, first) = lead.heard(basmala(3, "Locked"), 2.0);
        assert_eq!((h.loc, h.word, first), ((1, 1, 3), 2, true));
        // Not after a surah's last ayah, or only guessed: nothing.
        let mut lead = Lead::default();
        lead.heard(heard((113, 2, 1), 77_400, "Locked"), 0.0);
        lead.heard(takbir(5), 1.0);
        lead.heard(heard((113, 5, last), 77_410, "Locked"), 2.0);
        lead.heard(heard((0, 5, 1), 0, "Searching"), 3.0);
        assert_eq!(lead.takbir(), None);
    }

    #[test]
    fn juz_amma_surahs_lead_one_into_the_next() {
        // Al-Ikhlas (112, 4 ayahs) ends at its 4th ayah's last word; al-Falaq follows, then an-Nas.
        let mut lead = Lead::default();
        let last = words_in(112, 4).unwrap() as u16;
        lead.heard(heard((112, 4, last), 77_400, "Locked"), 0.0);
        let (h, _) = lead.heard(basmala(1, "Locked"), 1.0);
        assert_eq!((h.loc, h.word), ((113, 0, 1), 77_397));
        // The isti'adha in between ends the lead, and the basmala after it leads again: al-Ikhlas's end is still the
        // last place followed.
        lead.heard(heard((0, 1, 1), 77_429, "Locked"), 2.0);
        assert_eq!(lead.heard(basmala(2, "Locked"), 3.0), (Heard { loc: (113, 0, 2), word: 77_398, ..basmala(2, "Locked") }, true));
    }

    /// At-Tahrim's last words (66:12:15-17) on a line; al-Mulk's Bismillah line under it, as the reader lays it out, if
    /// `bismillah`; al-Mulk's first words under that. Words 50 wide, right to left, lines 40 apart.
    fn page(bismillah: bool) -> Vec<Slot> {
        let line = |y: f32, locs: &[Loc]| -> Vec<Slot> {
            (0..).zip(locs).map(|(k, &l)| (l, Rect::from_min_size(pos2(200.0 - 60.0 * k as f32, y), vec2(50.0, 40.0)), y + 28.0, 20.0)).collect()
        };
        let mut words = line(0.0, &[(66, 12, 15), (66, 12, 16), (66, 12, 17)]);
        if bismillah {
            words.extend(line(40.0, &[(67, 0, 1), (67, 0, 2), (67, 0, 3), (67, 0, 4)]));
        }
        words.extend(line(80.0, &[(67, 1, 1), (67, 1, 2)]));
        words
    }

    /// The marker after at-Tahrim's last word and al-Mulk's basmala up to its third word, and a second of frames over
    /// `words`: what each showed.
    fn basmala_heard_over(words: &[Slot]) -> Vec<Shown> {
        let (settings, ctx) = (Settings::default(), egui::Context::default());
        let colors = crate::theme::Colors::of(&quran_core::settings::Settings::default(), false);
        let params = Params::of(&settings);
        let mut m = Marker::default();
        m.heard(heard((66, 12, 17), 72_459, "Locked"), 0.0, &params);
        m.heard(basmala(3, "Locked"), 0.5, &params);
        (1..=60)
            .map(|k| {
                let now = 0.5 + f64::from(k) / 60.0;
                let mut shown = None;
                let mut output = ctx.run_pass(egui::RawInput { time: Some(now), ..Default::default() }, |ctx| {
                    let layer = |n| LayerId::new(egui::Order::Middle, egui::Id::unique(n));
                    let area = Rect::from_min_size(pos2(0.0, 0.0), vec2(300.0, 120.0));
                    let stage = Stage { now, dt: 1.0 / 60.0, area, layers: (layer("r"), layer("p")), scale: 1.0, scrolls: false };
                    shown = Some(m.frame(ctx, &stage, &settings, &colors, words));
                });
                output.textures_delta.clear();
                shown.unwrap()
            })
            .collect()
    }

    #[test]
    fn the_pointer_reads_the_basmala_on_the_next_surahs_bismillah_line() {
        let shown = basmala_heard_over(&page(true));
        let at = shown.last().and_then(|s| s.at).unwrap();
        // On the third of the line's words (x 80..130), and nothing to go to.
        assert!((80.0..=130.0).contains(&at.x) && (40.0..80.0).contains(&at.y), "{at:?}");
        assert!(shown.iter().all(|s| s.steer.lead.is_none() && s.steer.go_to.is_none()));
    }

    #[test]
    fn without_its_bismillah_line_the_pointer_waits_at_the_surahs_first_word() {
        let shown = basmala_heard_over(&page(false));
        // The first word's right end: where its reading starts.
        assert_eq!(shown.last().and_then(|s| s.at).map(|at| (at.x, at.y > 80.0)), Some((250.0, true)));
    }

    #[test]
    fn the_next_surah_off_screen_is_gone_to_at_once_and_once() {
        let shown = basmala_heard_over(&page(true)[..3]);
        assert_eq!(shown[0].steer.lead, Some(67), "the first frame, not after a wait");
        assert!(shown[1..].iter().all(|s| s.steer.lead.is_none() && s.steer.go_to.is_none() && s.at.is_none()));
    }
}
