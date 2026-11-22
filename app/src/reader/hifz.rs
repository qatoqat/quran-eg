//! Memorizing (hifz): the ayahs' words hidden where they are, the mushaf's signs between them kept (the rub el hizb,
//! pause marks, the sajdah sign). Turning the mode on or off fades the words, their meanings and the translations out
//! or in, and a word clicked fades in or out on its own. Nothing is laid out again: only what is drawn changes.

use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};

use egui::{Id, Response, Ui};
use quran_core::glyph::Marks;

use super::paint::Slot;
use crate::sheet::Part;

/// Seconds a fade takes.
pub const FADE: f32 = 0.2;
/// The most room a pause mark's glyph takes (in em): none, give or take a rounding in the font.
const FLAT: f32 = 0.05;

/// A word of an ayah: surah, ayah, word.
pub type Loc = (u16, u16, u16);

/// What memorizing hides, and what clicks have shown.
pub struct Veil {
    /// The mode's fade, kept by egui: one per view, so that a view opened while memorizing starts with its words hidden.
    id: Id,
    /// How hidden the words are this frame: 0, all shown … 1, memorizing.
    mode: f32,
    /// The words clicked to show them.
    revealed: HashSet<Loc>,
    /// Words clicked a moment ago: when, and how far shown they were then (they fade from there).
    flips: Vec<(Loc, f64, f32)>,
}

impl Default for Veil {
    fn default() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let id = Id::unique(("hifz", NEXT.fetch_add(1, Ordering::Relaxed)));
        Self { id, mode: 0.0, revealed: HashSet::new(), flips: Vec::new() }
    }
}

impl Veil {
    /// A new frame, memorizing being `on`: frames are asked for while something fades; once memorizing is off and
    /// faded out, the words shown are forgotten.
    pub fn frame(&mut self, ctx: &egui::Context, on: bool) {
        self.mode = ctx.animate_bool_with_time(self.id, on, FADE);
        let now = ctx.input(|i| i.time);
        self.flips.retain(|f| now - f.1 < f64::from(FADE));
        if !self.flips.is_empty() {
            ctx.request_repaint();
        }
        if self.mode == 0.0 {
            self.revealed.clear();
            self.flips.clear();
        }
    }

    /// What it hides at `now`, if anything.
    pub fn shade(&self, now: f64) -> Option<Shade<'_>> {
        (self.mode > 0.0).then_some(Shade { mode: self.mode, now, veil: self })
    }

    /// The word was clicked: it shows, or hides again (turning back from where a fade has it).
    fn flip(&mut self, at: Loc, now: f64) {
        let from = self.shown(at, now);
        if !self.revealed.remove(&at) {
            self.revealed.insert(at);
        }
        self.flips.retain(|f| f.0 != at);
        self.flips.push((at, now, from));
    }

    /// How far the word is shown by a click (0 … 1).
    fn shown(&self, at: Loc, now: f64) -> f32 {
        let target = if self.revealed.contains(&at) { 1.0 } else { 0.0 };
        match self.flips.iter().find(|f| f.0 == at) {
            Some(&(_, since, from)) => from + (target - from) * (((now - since) / f64::from(FADE)) as f32).clamp(0.0, 1.0),
            None => target,
        }
    }

    /// A click on a word (of this frame's `words`) shows it or hides it again; the hand shows over one.
    pub fn input(&mut self, ui: &Ui, pages: &Response, words: &[Slot]) {
        let word = |at: egui::Pos2| words.iter().find(|w| w.0.1 > 0 && w.1.contains(at)).map(|w| w.0);
        if pages.hover_pos().and_then(word).is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if pages.clicked()
            && let Some(at) = pages.interact_pointer_pos().and_then(word)
        {
            self.flip(at, ui.input(|i| i.time));
            ui.ctx().request_repaint();
        }
    }

    #[cfg(test)]
    pub fn revealed(&self) -> &HashSet<Loc> {
        &self.revealed
    }

    /// Show these words, and no others.
    #[cfg(test)]
    pub fn reveal(&mut self, words: impl IntoIterator<Item = Loc>) {
        self.revealed = words.into_iter().collect();
    }
}

/// How much memorizing hides at one frame, for painting.
#[derive(Clone, Copy)]
pub struct Shade<'a> {
    /// 0: nothing … 1: the ayahs' words (but those clicked), their meanings and the translations.
    mode: f32,
    now: f64,
    veil: &'a Veil,
}

impl Shade<'_> {
    /// How opaque the word's letters are.
    pub fn word(&self, at: Loc) -> f32 {
        1.0 - self.mode * (1.0 - self.veil.shown(at, self.now))
    }

    /// How opaque meanings and translations are.
    pub fn notes(&self) -> f32 {
        1.0 - self.mode
    }
}

/// Which of a word's glyphs (`parts`, in the order of its code) are its signs, as the QCF fonts draw them: the rub el
/// hizb is the first glyph and takes room, the pause marks and the sajdah sign are the last ones, marks taking none
/// (but the sajdah sign, which is last). The glyphs that open it and the glyphs that close it, if the font bears the
/// text out; `None` if not (a sign drawn as part of the letters, as in 68:1's nūn).
pub fn sign_parts(marks: &Marks, parts: &[Part]) -> Option<(usize, usize)> {
    let (opening, closing) = (usize::from(marks.opening > 0), usize::from(marks.closing_count));
    if parts.len() < opening + closing {
        return None;
    }
    let inked = |p: &Part| p.size[0] > 0.0 && p.size[1] > 0.0;
    let flat = |p: &Part| p.advance.abs() <= FLAT;
    let (head, rest) = parts.split_at(opening);
    let (letters, tail) = rest.split_at(rest.len() - closing);
    let opens = head.iter().all(|p| inked(p) && !flat(p)) && (opening == 0 || !letters.is_empty());
    let closes = tail.iter().all(inked) && tail.iter().rev().skip(1).all(flat);
    // Letters that take room, or a word that is nothing but its pause mark (40:77:5: its letters are the word before's).
    let word = letters.iter().any(|p| !flat(p)) || (opening == 0 && parts.iter().all(flat));
    (opens && closes && word).then_some((opening, closing))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn part(advance: f32, inked: bool) -> Part {
        let size = if inked { [0.4, 0.3] } else { [0.0, 0.0] };
        Part { uv: [0.0; 4], size, left: 0.0, top: 1.2, advance }
    }

    #[test]
    fn a_words_signs_are_its_glyphs_where_the_font_has_them() {
        let marks = |text: &str| Marks::of(text);
        // 2:2:4 (a word and its muʿānaqa dots), 36:52:6 (sīn and qalā), 2:39:7 (a mark the font gives a sliver of room).
        assert_eq!(sign_parts(&marks("رَيۡبَۛ"), &[part(1.2, true), part(0.0, true)]), Some((0, 1)));
        assert_eq!(sign_parts(&marks("مَّرۡقَدِنَاۜۗ"), &[part(2.2, true), part(0.0, true), part(0.0, true)]), Some((0, 2)));
        assert_eq!(sign_parts(&marks("ٱلنَّارِۖ"), &[part(1.5, true), part(0.008, true)]), Some((0, 1)));
        // The rub el hizb opens 22:60:1, which a pause mark closes; 7:206:11's sajdah sign takes room, after the line over its word.
        assert_eq!(sign_parts(&marks("۞ ذَٰلِكَۖ"), &[part(0.6, true), part(2.4, true), part(0.0, true)]), Some((1, 1)));
        assert_eq!(sign_parts(&marks("يَسۡجُدُونَۤ۩"), &[part(2.5, true), part(0.0, true), part(0.55, true)]), Some((0, 1)));
        // 40:77:5's glyph is its pause mark alone.
        assert_eq!(sign_parts(&marks("حَقّٞۚ"), &[part(0.0, true)]), Some((0, 1)));
        // The sign drawn with the letters: 68:1's nūn (a blank glyph after it), 3:78:23's one glyph; text the glyphs
        // do not bear out (a mark the printed mushaf does not have).
        assert_eq!(sign_parts(&marks("نٓۚ"), &[part(1.06, true), part(0.0, false)]), None);
        assert_eq!(sign_parts(&marks("ٱللَّهِۖ"), &[part(1.1, true)]), None);
        assert_eq!(sign_parts(&marks("۞ إِنَّ"), &[part(1.4, true)]), None);
        // No signs: none of its glyphs (a line over a word of prostration is the word's).
        assert_eq!(sign_parts(&marks("يَخِرُّونَۤ"), &[part(2.0, true), part(0.0, true)]), Some((0, 0)));
    }

    #[test]
    fn a_word_clicked_fades_in_and_back_from_where_it_is() {
        let mut veil = Veil::default();
        let at = (2, 2, 4);
        let shade = |veil: &Veil, now: f64| Shade { mode: 1.0, now, veil }.word(at);
        assert_eq!(shade(&veil, 0.0), 0.0);
        veil.flip(at, 1.0);
        assert!((shade(&veil, 1.1) - 0.5).abs() < 1e-3 && shade(&veil, 1.3) == 1.0);
        // Clicked again halfway: it fades out from half shown.
        veil.flip(at, 1.1);
        assert!((shade(&veil, 1.1) - 0.5).abs() < 1e-3 && (shade(&veil, 1.2) - 0.25).abs() < 1e-3 && shade(&veil, 1.4) == 0.0);
        // Half way through the mode's own fade, a hidden word is half there; meanings follow the mode.
        let half = Shade { mode: 0.5, now: 9.0, veil: &veil };
        assert_eq!((half.word(at), half.notes()), (0.5, 0.5));
    }
}
