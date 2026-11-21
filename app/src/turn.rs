//! The voice pointer turning to another place: the next page loads out of sight while the reader is on a page's last
//! lines, and a page turn or a far place fades the page out, brings in the view loaded meanwhile (once what it shows
//! is drawn, or after a short wait) and fades it in.

use quran_core::view::{Location, View};

use crate::reader::{Env, Reader};

/// Seconds the page fades out and in, and the longest the faded-out page waits for the view it goes to.
const OUT: f64 = 0.2;
const IN: f64 = 0.3;
const WAIT: f64 = 1.5;

/// Where the voice pointer takes the reader (in this view or another), and the surah whose head to show there.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Move {
    pub to: Location,
    pub head: Option<u16>,
}

#[derive(Default)]
pub struct Turn {
    /// A view loading out of sight: the next page, or the one the move goes to.
    next: Option<Reader>,
    /// The move under way, and when the page began to fade out for it.
    going: Option<(Move, f64)>,
    /// When the view came in, to fade in.
    came: Option<f64>,
}

impl Turn {
    /// How opaque the page is at `now`: fading out for a move, or in after it.
    pub fn opacity(&self, now: f64) -> f32 {
        let t = match (&self.going, self.came) {
            (Some((_, since)), _) => 1.0 - (now - since) / OUT,
            (None, Some(came)) => (now - came) / IN,
            (None, None) => 1.0,
        };
        t.clamp(0.0, 1.0) as f32
    }

    /// Whether a view came in a moment ago: a move asked for in it then is the same one, made without a fade.
    pub fn settling(&self, now: f64) -> bool {
        self.came.is_some_and(|came| now - came < IN + 1.0)
    }

    /// Whether a move is under way.
    pub fn going(&self) -> bool {
        self.going.is_some()
    }

    /// Whether the page is fading (frames are wanted).
    pub fn fading(&self, now: f64) -> bool {
        self.going.is_some() || self.came.is_some_and(|came| now - came < IN)
    }

    /// Loads `view` (the next page) out of sight, unless it is loading already or a move is under way.
    pub fn preload(&mut self, view: View, ctx: &egui::Context, env: &Env<'_>) {
        if self.going.is_none() && self.next.as_ref().is_none_or(|r| r.view != view) {
            self.next = quiet(Reader::open(view, None, ctx, env));
        }
    }

    /// The pointer asks to go to `to`, from view `here`: the page fades out while the view it goes to loads (unless it
    /// is this one, or loaded already). Asked during a move, it becomes that move's place (its fade goes on), keeping a
    /// surah's head asked for before when it goes to that surah's first ayah: the listener's guess at a surah's first
    /// word, then the basmala leading into it.
    pub fn start(&mut self, to: Move, here: View, now: f64, ctx: &egui::Context, env: &Env<'_>) {
        let Some((to, since)) = merged(self.going.as_ref(), to, now) else { return };
        if to.to.view == here {
            self.next = None;
        } else if let Some(next) = self.next.as_mut().filter(|r| r.view == to.to.view) {
            next.go_to(to.to.at);
        } else {
            self.next = quiet(Reader::open(to.to.view, to.to.at, ctx, env));
        }
        (self.going, self.came) = (Some((to, since)), None);
    }

    /// The move to make at `now`, once the page has faded out and the view it goes to is ready (or has been waited
    /// for long enough): the move, and that view (none for a move in this one).
    pub fn due(&mut self, now: f64) -> Option<(Move, Option<Reader>)> {
        let since = self.going.as_ref()?.1;
        let ready = self.next.as_ref().is_none_or(Reader::ready) || now - since >= OUT + WAIT;
        if now - since < OUT || !ready {
            return None;
        }
        let (to, _) = self.going.take()?;
        self.came = Some(now);
        let mut next = self.next.take();
        if let Some(reader) = next.as_mut() {
            reader.set_quiet(false);
        }
        Some((to, next))
    }

    /// The view loading out of sight, while it loads: it is drawn unseen, as drawing lays it out and asks for its glyphs.
    pub fn loading(&mut self) -> Option<Reader> {
        self.next.take_if(|r| !r.ready())
    }

    /// Back after drawing it.
    pub fn loaded(&mut self, reader: Option<Reader>) {
        if reader.is_some() {
            self.next = reader;
        }
    }

    /// The pages of the view loading out of sight, whose fonts the view shown must not free.
    pub fn pages(&self) -> Vec<u16> {
        self.next.as_ref().map_or_else(Vec::new, Reader::pages)
    }

    /// The reader went elsewhere themselves: nothing is under way, nothing loads.
    pub fn cancel(&mut self) {
        *self = Turn::default();
    }
}

/// The move `to`, asked for at `now` while `going` is under way (the move and when its fade began), and when its fade
/// began; none for the same move again (asked every frame while its place is off screen).
fn merged(going: Option<&(Move, f64)>, mut to: Move, now: f64) -> Option<(Move, f64)> {
    let Some((before, since)) = going else { return Some((to, now)) };
    let opens = |c: u16| to.to.at.is_some_and(|w| w.chapter == c && w.verse == 1);
    to.head = to.head.or(before.head.filter(|&c| opens(c)));
    (to != *before).then_some((to, *since))
}

fn quiet(reader: Option<Reader>) -> Option<Reader> {
    reader.map(|mut r| {
        r.set_quiet(true);
        r
    })
}


#[cfg(test)]
mod tests {
    use quran_core::view::WordRef;

    use super::*;

    fn to(chapter: u16, verse: u16, head: Option<u16>) -> Move {
        Move { to: View::Surah { chapter, span: None }.at(Some(WordRef { chapter, verse, word: 1 })), head }
    }

    #[test]
    fn a_move_under_way_takes_the_latest_ask_and_keeps_a_surahs_head() {
        assert_eq!(merged(None, to(82, 1, None), 2.0), Some((to(82, 1, None), 2.0)), "a move of its own, faded from now");
        // As heard live: 82:1:1 guessed as the basmala begins, then the basmala leading into al-Infitar, then 82:1:1.
        let going = (to(82, 1, None), 1.0);
        assert_eq!(merged(Some(&going), to(82, 1, Some(82)), 1.1), Some((to(82, 1, Some(82)), 1.0)), "the head, in the same fade");
        let going = (to(82, 1, Some(82)), 1.0);
        assert_eq!(merged(Some(&going), to(82, 1, None), 1.2), None, "the surah's first ayah keeps its head: the same move");
        assert_eq!(merged(Some(&going), to(82, 5, None), 1.2), Some((to(82, 5, None), 1.0)), "another ayah does not");
    }
}
