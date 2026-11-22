//! Words the reading went past unread: words or whole ayahs skipped. The furthest word the followed reports (Locked,
//! Drifting or Paused) have reached is kept; a confirmed move on from it (a held jump once the hold lets it go, from
//! where it was first heard), within about a page (further is navigation), marks the words between when they are the
//! fewest set or more and the reader could not have read them since: at their pace from any report of the last few
//! seconds (or the furthest word's first, while the reading stays there), silences left out, and two words more.
//! The listener lags and catches up (a word held for over a second, then two or three at once: about once a minute in
//! the user's sessions), and those are not skips; nor are words a report the hold kept back was on (the listener heard
//! the reader there). Gone back behind the furthest word and read on a few words from there, the reader is reading
//! again, and a skip in that reading is one too (the user's page 7: 2:40 read again, then on with 2:42): moves are
//! measured from that reading's furthest word, but a move on to where the reading had got to before (or two words short
//! of it) or past it from there: the reader goes on from where they were, or the listener, back for a moment (mostly a
//! report or two), comes back to them. A later report on a marked word takes its mark away, with the words
//! since the report before when that was a step of reading, and a step back onto a run of marks a word or two in takes
//! the run's first words too (the listener finds the place a little late): reading them clears them. They are drawn in
//! the accuracy range's shapes under the pointer, and go when listening stops or the reader dismisses them.

use std::collections::{BTreeMap, VecDeque};

use egui::{Rect, pos2};
use quran_core::reference::{page_of, verses_in, words_in};
use quran_core::view::WordRef;

use super::hold::PAGE;
use super::keep::edges;
use super::link::{Heard, Loc};
use super::marker::position;
use super::motion::Reading;
use crate::reader::Slot;

/// Seconds back the reports the reach is measured from go (as far as the follower's pace looks).
const WINDOW: f64 = 4.0;
/// The least pace the reader is taken to read at (words a second), and the words more the reach allows: the listener's
/// lag as it catches up. Replayed on the user's sessions (about an hour, 30 gaps of two words or more), what is left
/// is the listener's three jumps of 19-26 words; the catch-ups clear the reach by 0.6 words or more.
const LEAST_PACE: f32 = 1.0;
const LAG: f32 = 2.0;
/// Seconds a silence the voice's level shows must last to count (a breath between words is reading time); one the
/// listener reports (Paused) counts whole.
const QUIET: f64 = 0.5;
/// A step back onto a run of marks this many words in, or fewer, takes the run's first words too.
const BACK_IN: u32 = 2;
/// Words a reading gone back behind the furthest word reads on from the furthest back it went that make it a reading
/// again, and the words short of where the reading had got to before that a move on reaches it.
const AGAIN: u32 = 3;
const NEAR: u32 = 2;

/// A report as the marks go by it: its word (index in the whole Quran, and place), its position (the index plus the
/// share read) and the clock time it came at.
#[derive(Debug, Clone, Copy, PartialEq)]
struct At {
    i: u32,
    loc: Loc,
    pos: f32,
    time: f64,
}

/// What the toast says of the marks: the first run of them (its first and last place, and how many words), and how many
/// runs more.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Summary {
    pub from: Loc,
    pub to: Loc,
    pub words: u32,
    pub others: usize,
}

#[derive(Debug, Default)]
pub struct Skipped {
    /// The words marked: place, and index in the whole Quran.
    marks: BTreeMap<Loc, u32>,
    /// The first report on the furthest word reached, and whether the reading has stayed there since.
    furthest: Option<At>,
    stayed: bool,
    /// Gone back behind it, the furthest back the reading went since; reading again, the furthest word it had reached
    /// before, while the reading is behind it.
    back: Option<At>,
    before: Option<At>,
    /// The last report followed, and those of the last `WINDOW` s.
    last: Option<At>,
    recent: VecDeque<At>,
    /// Where a held jump was first heard that the hold let go on an unsure report (the next followed one arrives there).
    arrived: Option<At>,
    /// The reports of the last `WINDOW` s the hold kept back: the words they are on were heard, followed or not.
    held_back: VecDeque<At>,
    /// The silences of late (clock time from, to), and since when the reader has been stopped where the last report is
    /// (Paused) and the voice quiet.
    silences: VecDeque<(f64, f64)>,
    paused: Option<f64>,
    quiet: Option<f64>,
    /// The reader moved the view away from the furthest word: the next move past it is theirs, no skip.
    chosen: bool,
}

/// The place `n` words on from `loc` (back, for a negative `n`), across ayahs and surahs, the words counted as the
/// listener counts them (no ayah ends).
fn step(loc: Loc, n: i64) -> Option<Loc> {
    let (mut s, mut a) = (loc.0, loc.1);
    let mut w = i64::from(loc.2) + n;
    loop {
        if w < 1 {
            if a > 1 {
                a -= 1;
            } else {
                s = s.checked_sub(1).filter(|&s| s > 0)?;
                a = verses_in(s)?;
            }
            w += i64::try_from(words_in(s, a)?).ok()?;
        } else {
            let len = i64::try_from(words_in(s, a)?).ok()?;
            if w <= len {
                return Some((s, a, u16::try_from(w).ok()?));
            }
            w -= len;
            if a < verses_in(s)? { a += 1 } else { (s, a) = (s + 1, 1) }
        }
    }
}

impl Skipped {
    /// A report the marker took at clock time `now`: `h` as it took it (`reading` its state), `held` where and when a
    /// jump the hold let go on it was first heard, `pace` the reader's pace before it (words a second), `min` the fewest
    /// words gone past unread that are marked (`None`: none are). Formulas and basmala places say nothing of the ayahs'
    /// words.
    pub fn heard(&mut self, h: &Heard, reading: Reading, held: Option<(f32, f64)>, pace: f32, min: Option<u32>, now: f64) {
        if h.loc.0 == 0 || h.loc.1 == 0 {
            return;
        }
        let at = At { i: h.word, loc: h.loc, pos: position(h), time: now };
        let first = held.and_then(|(pos, time)| {
            let i = pos.max(0.0) as u32;
            let loc = step(h.loc, i64::from(i) - i64::from(h.word)).filter(|_| i <= h.word)?;
            Some(At { i, loc, pos, time })
        });
        // Any report but a Paused one: the reader sounds.
        match (reading == Reading::Paused, self.paused) {
            (true, None) => self.paused = Some(now),
            (false, Some(from)) => {
                self.silences.push_back((from, now));
                self.paused = None;
            }
            _ => {}
        }
        if reading == Reading::Tentative {
            self.arrived = first.or(self.arrived);
            return;
        }
        let arrived = first.or(self.arrived.take()).filter(|a| a.i <= at.i).unwrap_or(at);
        self.read(arrived, at, pace, min.unwrap_or(1));
        self.skip(arrived, pace, min);
        self.went(arrived, at, now);
    }

    /// A report the hold kept back at clock time `now`: no move, but its word was heard (a move on past it is measured
    /// from there).
    pub fn held_back(&mut self, h: &Heard, now: f64) {
        if h.loc.0 > 0 && h.loc.1 > 0 {
            self.held_back.push_back(At { i: h.word, loc: h.loc, pos: position(h), time: now });
        }
    }

    /// The voice came or went (the link's presence) at clock time `now`: back, the reader sounds again (a stop the
    /// listener reported is over too).
    pub fn voice(&mut self, quiet: bool, now: f64) {
        if quiet {
            self.quiet = self.quiet.or(Some(now));
            return;
        }
        for from in [self.quiet.take().filter(|from| now - from >= QUIET), self.paused.take()].into_iter().flatten() {
            self.silences.push_back((from, now));
        }
    }

    /// The reader moved the view themselves, now showing `words`: away from the furthest word read, the next move past it
    /// is theirs.
    pub fn moved(&mut self, words: &[Slot]) {
        self.chosen = self.furthest.is_some_and(|f| words.binary_search_by(|w| w.0.cmp(&f.loc)).is_err());
    }

    /// Every mark goes (the reader dismissed them, or they are not shown).
    pub fn clear(&mut self) {
        self.marks.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.marks.is_empty()
    }

    /// The first run of marks, and how many more there are.
    pub fn summary(&self) -> Option<Summary> {
        let mut runs = 0;
        let mut first: Option<(Loc, Loc, u32)> = None;
        let mut prev: Option<u32> = None;
        for (&loc, &i) in &self.marks {
            if prev.is_none_or(|p| p + 1 != i) {
                runs += 1;
            }
            if runs == 1 {
                let (from, ..) = first.unwrap_or((loc, loc, 0));
                first = Some((from, loc, first.map_or(1, |f| f.2 + 1)));
            }
            prev = Some(i);
        }
        let (from, to, words) = first?;
        Some(Summary { from, to, words, others: runs - 1 })
    }

    /// Where Go back takes the reader: just before the first run of marks (the word read before the mistake) on its page
    /// and in its surah, else the run's first word.
    pub fn back_to(&self) -> Option<WordRef> {
        let (&from, _) = self.marks.first_key_value()?;
        let before = step(from, -1).filter(|b| b.0 == from.0 && page_of(b.0, b.1) == page_of(from.0, from.1));
        let (chapter, verse, word) = before.unwrap_or(from);
        Some(WordRef { chapter, verse, word })
    }

    /// Rectangles over the marked words among `words` (those on screen, in reading order): one over each run of them on
    /// a line, `size` tenths of the line's height tall around where the pointer is there (`lift` of the text size over
    /// the baseline), reaching beyond the words as the range does.
    pub fn rects(&self, words: &[Slot], lift: f32, size: u8) -> Vec<Rect> {
        let (Some((&first, _)), Some((&last, _))) = (self.marks.first_key_value(), self.marks.last_key_value()) else {
            return Vec::new();
        };
        let (lo, hi) = (words.partition_point(|w| w.0 < first), words.partition_point(|w| w.0 <= last));
        let marked = |k: usize| self.marks.contains_key(&words[k].0);
        let mut out = Vec::new();
        let mut k = lo;
        while k < hi {
            if !marked(k) {
                k += 1;
                continue;
            }
            let (_, rect, baseline, em) = words[k];
            let on_line = |j: usize| (words[j].1.center().y - rect.center().y).abs() < rect.height() * 0.5;
            let mut end = k;
            while end + 1 < hi && marked(end + 1) && on_line(end + 1) {
                end += 1;
            }
            let (left, right) = edges(words, (k, end));
            let (height, pad, y) = (rect.height() * f32::from(size) / 10.0, rect.height() * 0.12, baseline - lift * em);
            out.push(Rect::from_min_max(pos2(left - pad, y - height / 2.0), pos2(right + pad, y + height / 2.0)));
            k = end + 1;
        }
        out
    }

    /// The words the reading shows read lose their marks: from where it arrived to the report's word, with the words
    /// since the last report when that was a step of reading (fewer than `min` between, or no more than the reader could
    /// have read since), and, stepping back a word or two into a run of marks, the run's first words.
    fn read(&mut self, arrived: At, at: At, pace: f32, min: u32) {
        if self.marks.is_empty() {
            return;
        }
        let mut from = arrived.i;
        match self.last {
            Some(last) if last.i < arrived.i => {
                if arrived.i - last.i - 1 < min || arrived.i as f32 <= self.reach(arrived.time, pace, arrived.i) {
                    from = last.i + 1;
                }
            }
            Some(last) if arrived.i < last.i => {
                if let Some(start) = self.run_start(arrived)
                    && arrived.i - start <= BACK_IN
                {
                    from = start;
                }
            }
            _ => {}
        }
        self.marks.retain(|_, i| *i < from || *i > at.i);
    }

    /// Marks the words between the furthest reached (where the reading had got to before it went back, for a move on to
    /// there or past it) and where the reading arrived: `min` or more (`None`: none are marked), within a page, further
    /// than the reader could have read since, and not a move from a view they chose.
    fn skip(&mut self, arrived: At, pace: f32, min: Option<u32>) {
        let from = self.before.filter(|b| arrived.i + NEAR >= b.i).or(self.furthest);
        let Some(f) = from.filter(|f| arrived.i > f.i) else { return };
        let chosen = std::mem::take(&mut self.chosen);
        let Some(min) = min else { return };
        // (From the furthest word short of the arrival a report the hold kept back was on.)
        let f = self.held_back.iter().filter(|k| k.i > f.i && k.i < arrived.i).max_by_key(|k| k.i).map_or(f, |k| *k);
        let between = arrived.i - f.i - 1;
        if chosen || between < min || arrived.i - f.i > PAGE as u32 || arrived.i as f32 <= self.reach(arrived.time, pace, arrived.i) {
            return;
        }
        // Their places, from the furthest word's on: marked only when they come out at the arrival's.
        let mut words = Vec::with_capacity(between as usize);
        let mut loc = Some(f.loc);
        for i in f.i + 1..arrived.i {
            loc = loc.and_then(|l| step(l, 1));
            words.extend(loc.map(|l| (l, i)));
        }
        if loc.and_then(|l| step(l, 1)) == Some(arrived.loc) {
            self.marks.extend(words);
        }
    }

    /// The reading arrived at `arrived` and is at `at` (clock time `now`): the furthest word and the last report move on;
    /// gone a page or more away (navigation), the furthest word is where it went; gone back behind it and read on
    /// `AGAIN` words from the furthest back since, it is where this reading again is.
    fn went(&mut self, arrived: At, at: At, now: f64) {
        let page = PAGE as u32;
        let navigated = self.last.is_some_and(|l| arrived.i + page < l.i) || self.furthest.is_some_and(|f| arrived.i > f.i + page);
        match self.furthest {
            // At the furthest word or the one before it.
            Some(f) if !navigated && at.i <= f.i && at.i + 1 >= f.i => self.back = None,
            Some(f) if !navigated && at.i <= f.i => {
                let back = self.back.filter(|b| b.i <= arrived.i).unwrap_or(arrived);
                (self.stayed, self.back) = (false, Some(back));
                if at.i >= back.i + AGAIN {
                    self.before = self.before.filter(|b| b.i > f.i).or(Some(f));
                    (self.furthest, self.stayed, self.back) = (Some(at), true, None);
                }
            }
            _ => {
                (self.furthest, self.stayed, self.back) = (Some(if arrived.i == at.i { arrived } else { at }), true, None);
                self.chosen &= !navigated;
                self.before = self.before.filter(|_| !navigated);
            }
        }
        self.before = self.before.filter(|b| at.i < b.i);
        for a in [arrived, at] {
            if self.recent.back() != Some(&a) {
                self.recent.push_back(a);
            }
        }
        while self.recent.front().is_some_and(|r| now - r.time > WINDOW) {
            self.recent.pop_front();
        }
        while self.held_back.front().is_some_and(|r| now - r.time > WINDOW) {
            self.held_back.pop_front();
        }
        let kept = self.furthest.filter(|_| self.stayed).map_or(now - WINDOW, |f| f.time.min(now - WINDOW));
        while self.silences.front().is_some_and(|s| s.1 < kept) {
            self.silences.pop_front();
        }
        self.last = Some(at);
    }

    /// The furthest the reader could have read to by clock time `to` at `pace`, for a move to word `on`: from any
    /// report of the last `WINDOW` s (the last one, if none; those the hold kept back short of `on` too), or the
    /// furthest word's first while the reading stays there, their silences left out, and `LAG` words more.
    fn reach(&self, to: f64, pace: f32, on: u32) -> f32 {
        let pace = pace.max(LEAST_PACE);
        let recent = self.recent.iter().chain(self.held_back.iter().filter(|a| a.i < on)).filter(|a| to - a.time <= WINDOW);
        let anchors = recent.chain(self.last.iter()).chain(self.furthest.iter().filter(|_| self.stayed));
        anchors.map(|a| a.pos + pace * self.sounding(a.time.min(to), to)).fold(f32::NEG_INFINITY, f32::max) + LAG
    }

    /// Seconds from `from` to `to` (clock time) the reader was not silent: the silences (and those going on: a quiet
    /// voice once it is a silence) inside it, overlapping ones once.
    fn sounding(&self, from: f64, to: f64) -> f32 {
        let going = [self.paused, self.quiet.filter(|q| to - q >= QUIET)].into_iter().flatten().map(|a| (a, to));
        let clipped = self.silences.iter().copied().chain(going).map(|(a, b)| (a.max(from), b.min(to)));
        let mut inside: Vec<(f64, f64)> = clipped.filter(|(a, b)| b > a).collect();
        inside.sort_by(|x, y| x.0.total_cmp(&y.0));
        let (mut silent, mut end) = (0.0, from);
        for (a, b) in inside {
            silent += (b - a.max(end)).max(0.0);
            end = end.max(b);
        }
        ((to - from) - silent).max(0.0) as f32
    }

    /// The first word of the run of marks (marked words one after another) `a` is in, if it is marked.
    fn run_start(&self, a: At) -> Option<u32> {
        let mut start = *self.marks.get(&a.loc)?;
        for (_, &i) in self.marks.range(..a.loc).rev() {
            if i + 1 != start {
                break;
            }
            start = i;
        }
        Some(start)
    }
}

#[cfg(test)]
mod tests;
