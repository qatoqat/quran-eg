use egui::vec2;
use quran_core::settings::Voice as Settings;

use super::super::marker::Marker;
use super::super::motion::Params;
use super::*;

/// Reports every 0.32 s, two a word, as the stand-in listener sends them: a word every 0.64 s.
const PACE: f32 = 1.0 / 0.64;

/// The index in the whole Quran of the word at `loc`, as the listener counts (none for a formula or a basmala place).
fn index(loc: Loc) -> u32 {
    if loc.0 == 0 || loc.1 == 0 {
        return 0;
    }
    let words = |s: u16, a: u16| words_in(s, a).unwrap();
    let before: usize = (1..loc.0).flat_map(|s| (1..=verses_in(s).unwrap()).map(move |a| words(s, a))).sum();
    let here: usize = (1..loc.1).map(|a| words(loc.0, a)).sum();
    (before + here + usize::from(loc.2) - 1) as u32
}

fn report(loc: Loc, letter: u16, state: &str) -> Heard {
    Heard { loc, word: index(loc), letter, letters: 4, state: state.into(), conf: 0.97, speed: 8.2, t: 0.0, slid: None, alts: Vec::new(), plan: None }
}

fn reading(state: &str) -> Reading {
    match state {
        "Paused" => Reading::Paused,
        "Drifting" => Reading::Drifting,
        "Tentative" => Reading::Tentative,
        _ => Reading::Locked,
    }
}

/// A reader at the stand-in's pace, heard by `Skipped` alone (no hold: every report is followed).
struct Reader {
    s: Skipped,
    t: f64,
    min: Option<u32>,
}

impl Reader {
    fn new() -> Reader {
        Reader { s: Skipped::default(), t: 0.0, min: Some(2) }
    }

    /// A report 0.32 s after the last.
    fn hear(&mut self, loc: Loc, letter: u16, state: &str) {
        self.t += 0.32;
        self.s.heard(&report(loc, letter, state), reading(state), None, PACE, self.min, self.t);
    }

    /// `words` words read from `loc` on, two reports a word.
    fn read(&mut self, loc: Loc, words: i64) {
        for k in 0..words {
            let at = step(loc, k).unwrap();
            self.hear(at, 0, "Locked");
            self.hear(at, 2, "Locked");
        }
    }

    fn marked(&self) -> Vec<Loc> {
        self.s.marks.keys().copied().collect()
    }
}

/// The words from `from` to `to`.
fn span(from: Loc, to: Loc) -> Vec<Loc> {
    let mut out = vec![from];
    while *out.last().unwrap() != to {
        out.push(step(*out.last().unwrap(), 1).unwrap());
    }
    out
}

#[test]
fn places_step_across_ayahs_and_surahs_as_the_listener_counts_words() {
    assert_eq!((index((1, 1, 1)), index((68, 43, 1)), index((2, 94, 3))), (0, 73_012, 1595));
    assert_eq!((step((68, 44, 10), 1), step((68, 45, 1), -1)), (Some((68, 45, 1)), Some((68, 44, 10))));
    assert_eq!((step((68, 52, 5), 1), step((69, 1, 1), -1)), (Some((69, 1, 1)), Some((68, 52, 5))));
    assert_eq!(step((68, 43, 1), 26), Some((68, 46, 1)));
    assert_eq!((step((1, 1, 1), -1), step((114, 6, 3), 1)), (None, None));
}

#[test]
fn an_ayah_skipped_after_a_pause_is_marked_whole() {
    let mut r = Reader::new();
    // 68:43 and 68:44, a stop at its end (the listener says so), and on 2 s later with 68:46: 68:45 left out.
    r.read((68, 43, 1), 21);
    r.hear((68, 44, 10), 3, "Paused");
    r.t += 2.0;
    r.read((68, 46, 1), 4);
    assert_eq!(r.marked(), span((68, 45, 1), (68, 45, 5)));
    assert_eq!(r.s.summary(), Some(Summary { from: (68, 45, 1), to: (68, 45, 5), words: 5, others: 0 }));
    // Go back: to the word read before it, on the same page.
    assert_eq!(r.s.back_to(), Some(WordRef { chapter: 68, verse: 44, word: 10 }));
}

#[test]
fn words_skipped_inside_an_ayah_are_marked_from_the_fewest_set() {
    for (min, marked) in [(2, true), (3, true), (4, false)] {
        let mut r = Reader { min: Some(min), ..Reader::new() };
        // 68:43:1-5, then on with its 9th word: three left out, quicker than they could be read.
        r.read((68, 43, 1), 5);
        r.read((68, 43, 9), 3);
        let want = if marked { span((68, 43, 6), (68, 43, 8)) } else { Vec::new() };
        assert_eq!(r.marked(), want, "fewest {min}");
    }
    // Two left out as quickly are within what the listener lags by: no skip.
    let mut r = Reader::new();
    r.read((68, 43, 1), 5);
    r.read((68, 43, 8), 3);
    assert!(r.s.is_empty(), "{:?}", r.marked());
    // Marking off: nothing.
    let mut r = Reader { min: None, ..Reader::new() };
    r.read((68, 43, 1), 5);
    r.read((68, 43, 10), 3);
    assert!(r.s.is_empty());
}

#[test]
fn reading_the_marked_words_takes_their_marks_away() {
    let mut r = Reader::new();
    r.read((68, 43, 1), 21);
    r.hear((68, 44, 10), 3, "Paused");
    r.t += 2.0;
    r.read((68, 46, 1), 4);
    // Back to 68:45: the listener finds the reader a word in (the first word goes with it), then two words on at once.
    r.hear((68, 45, 2), 1, "Locked");
    assert_eq!(r.marked(), span((68, 45, 3), (68, 45, 5)));
    r.hear((68, 45, 3), 0, "Locked");
    r.hear((68, 45, 5), 1, "Locked");
    assert!(r.s.is_empty() && r.s.summary().is_none(), "{:?}", r.marked());
    // On to where they were and past it: nothing new.
    r.read((68, 46, 1), 12);
    assert!(r.s.is_empty());
    // Going back well into a run leaves its first words marked.
    r.read((68, 48, 1), 21);
    r.hear((68, 49, 10), 3, "Paused");
    r.t += 2.0;
    r.read((68, 51, 1), 2);
    assert_eq!(r.marked(), span((68, 50, 1), (68, 50, 5)));
    r.hear((68, 50, 4), 0, "Locked");
    assert_eq!(r.marked(), [span((68, 50, 1), (68, 50, 3)), vec![(68, 50, 5)]].concat());
    assert_eq!(r.s.summary(), Some(Summary { from: (68, 50, 1), to: (68, 50, 3), words: 3, others: 1 }));
    // Dismissed: all gone.
    r.s.clear();
    assert!(r.s.summary().is_none());
}

#[test]
fn a_pause_only_the_voice_heard_is_silence_too() {
    for voice in [false, true] {
        let mut r = Reader::new();
        r.read((68, 43, 1), 21);
        // A 3 s stop at 68:44's end that the listener does not report; the voice's level says it.
        if voice {
            r.s.voice(true, r.t + 0.3);
            r.s.voice(false, r.t + 3.3);
        }
        r.t += 3.3;
        r.read((68, 46, 1), 4);
        // Without it the stop is time the reader could have read 68:45 in.
        assert_eq!(r.s.is_empty(), !voice, "voice {voice}: {:?}", r.marked());
    }
}

#[test]
fn far_moves_formulas_and_moves_of_the_readers_own_mark_nothing() {
    let mut r = Reader::new();
    r.read((68, 43, 1), 4);
    // More than a page ahead (navigation), and reading on there.
    r.read((70, 1, 1), 4);
    assert!(r.s.is_empty() && index((70, 1, 1)) > index((68, 43, 4)) + 150);
    // A basmala's places and the formulas say nothing; al-Haaqqah's first ayah follows al-Qalam's last.
    let mut r = Reader::new();
    r.read((68, 51, 1), 17);
    r.s.heard(&Heard { word: 73_091, ..report((69, 0, 2), 0, "Locked") }, Reading::Locked, None, PACE, Some(2), r.t + 0.3);
    r.s.heard(&Heard { word: 77_430, ..report((0, 2, 1), 0, "Locked") }, Reading::Locked, None, PACE, Some(2), r.t + 0.6);
    r.t += 0.6;
    r.read((69, 1, 1), 3);
    assert!(r.s.is_empty(), "{:?}", r.marked());
    // The reader moved the view away from where the reading was: the next move past it is theirs.
    let slot = |loc: Loc| (loc, Rect::from_min_size(pos2(0.0, 0.0), vec2(50.0, 40.0)), 28.0, 20.0);
    for on_screen in [false, true] {
        let mut r = Reader::new();
        r.read((68, 43, 1), 8);
        r.s.moved(&[slot((68, 43, 7)), slot(if on_screen { (68, 43, 8) } else { (68, 43, 9) })]);
        r.read((68, 44, 3), 3);
        assert_eq!(r.s.is_empty(), !on_screen, "on screen {on_screen}");
    }
}

#[test]
fn the_marks_are_drawn_over_each_run_of_them_on_a_line() {
    // Two lines of three words, right to left, baselines 28 below each line's top, text size 20.
    let words: Vec<Slot> = span((68, 44, 9), (68, 45, 4))
        .into_iter()
        .enumerate()
        .map(|(k, loc)| {
            let (x, y) = (200.0 - 60.0 * (k % 3) as f32, 40.0 * (k / 3) as f32);
            (loc, Rect::from_min_size(pos2(x, y), vec2(50.0, 40.0)), y + 28.0, 20.0)
        })
        .collect();
    let mut s = Skipped::default();
    s.marks.extend(span((68, 45, 1), (68, 45, 3)).into_iter().map(|l| (l, index(l))));
    // 68:45:1 at the first line's end; 68:45:2-3 at the second line's start. Height 0.8 of the line around the pointer's
    // height (0.2 of the text size over the baseline), 0.12 of the line beyond the words.
    let rects = s.rects(&words, 0.2, 8);
    let want = [Rect::from_min_max(pos2(75.2, 8.0), pos2(134.8, 40.0)), Rect::from_min_max(pos2(135.2, 48.0), pos2(254.8, 80.0))];
    let close = |a: &Rect, b: &Rect| (a.min - b.min).length() < 1e-3 && (a.max - b.max).length() < 1e-3;
    assert!(rects.len() == 2 && rects.iter().zip(&want).all(|(a, b)| close(a, b)), "{rects:?}");
    assert!(Skipped::default().rects(&words, 0.2, 8).is_empty());
}

/// The marker, tuned (a jump ahead is held), hearing `reports` (place, letter, state, clock time).
fn marker(reports: &[(Loc, u16, &str, f64)]) -> Marker {
    let params = Params::of(&Settings::default());
    let mut m = Marker::default();
    for &(loc, letter, state, t) in reports {
        let h = Heard { t: t as f32, speed: 8.25, ..report(loc, letter, state) };
        m.heard(h, t, &params);
    }
    m
}

/// The stand-in's reading for the marker: `words` words from `loc` from clock time `t` on.
fn reads(loc: Loc, words: i64, t: &mut f64) -> Vec<(Loc, u16, &'static str, f64)> {
    let mut out = Vec::new();
    for k in 0..words {
        for letter in [0, 2] {
            *t += 0.32;
            out.push((step(loc, k).unwrap(), letter, "Locked", *t));
        }
    }
    out
}

#[test]
fn a_held_jump_marks_nothing_until_the_hold_lets_it_go_from_where_it_was_first_heard() {
    let mut t = 0.0;
    // A misreading the listener follows for two reports, twelve words on, then back: nothing.
    let mut news = reads((68, 43, 1), 5, &mut t);
    news.extend([((68, 44, 6), 0, "Locked", t + 0.32), ((68, 44, 6), 2, "Locked", t + 0.64)]);
    t += 0.64;
    news.extend(reads((68, 43, 6), 2, &mut t));
    let mut m = marker(&news);
    assert!(m.skipped().is_empty());
    // A real skip, read on from: held two reports, then marked up to where it was first heard.
    news.extend(reads((68, 44, 5), 3, &mut t));
    let mut m = marker(&news);
    assert_eq!(m.skipped().summary(), Some(Summary { from: (68, 43, 8), to: (68, 44, 4), words: 8, others: 0 }));
}

#[test]
fn the_listener_catching_up_is_no_skip() {
    // From the user's sessions: a word held over a second, then three words on at once (2:94:6 to 2:94:9, 2:111:12 to
    // 2:111:15), all Locked, the user reading on.
    let logged: &[(f64, Loc, u16, u16, &str)] = &[
        (350.08, (2, 93, 25), 1, 2, "Locked"), (350.4, (2, 93, 25), 0, 2, "Paused"), (350.72, (2, 93, 26), 0, 3, "Locked"),
        (351.04, (2, 93, 26), 1, 3, "Locked"), (351.68, (2, 93, 26), 2, 3, "Locked"), (352.0, (2, 93, 27), 0, 6, "Locked"),
        (352.96, (2, 93, 27), 5, 6, "Locked"), (353.92, (2, 94, 2), 1, 2, "Locked"), (354.56, (2, 94, 3), 2, 4, "Locked"),
        (354.88, (2, 94, 3), 3, 4, "Locked"), (355.2, (2, 94, 5), 3, 5, "Locked"), (355.52, (2, 94, 5), 4, 5, "Locked"),
        (355.84, (2, 94, 6), 2, 6, "Locked"), (356.16, (2, 94, 6), 3, 6, "Locked"), (356.48, (2, 94, 6), 4, 6, "Locked"),
        (357.12, (2, 94, 6), 3, 6, "Locked"), (357.44, (2, 94, 9), 0, 5, "Locked"), (357.76, (2, 94, 9), 3, 5, "Locked"),
        (358.08, (2, 94, 10), 0, 2, "Locked"),
        (619.52, (2, 111, 5), 1, 3, "Locked"), (619.84, (2, 111, 6), 1, 2, "Locked"), (620.16, (2, 111, 7), 0, 3, "Locked"),
        (620.48, (2, 111, 8), 0, 3, "Locked"), (620.8, (2, 111, 8), 2, 3, "Locked"), (621.12, (2, 111, 10), 1, 5, "Locked"),
        (621.44, (2, 111, 10), 4, 5, "Locked"), (621.76, (2, 111, 11), 0, 3, "Locked"), (622.08, (2, 111, 11), 2, 3, "Locked"),
        (622.4, (2, 111, 12), 0, 7, "Locked"), (622.72, (2, 111, 12), 4, 7, "Locked"), (623.04, (2, 111, 12), 5, 7, "Locked"),
        (623.68, (2, 111, 12), 6, 7, "Locked"), (624.0, (2, 111, 15), 5, 7, "Locked"), (624.32, (2, 111, 16), 1, 2, "Locked"),
    ];
    let params = Params::of(&Settings::default());
    let mut m = Marker::default();
    for &(t, loc, letter, letters, state) in logged {
        m.heard(Heard { letter, letters, t: t as f32, speed: 8.25, ..report(loc, 0, state) }, t, &params);
    }
    assert!(m.skipped().is_empty(), "{:?}", m.skipped().summary());
}

#[test]
fn reading_again_a_skip_is_marked_and_going_on_from_where_the_reading_had_got_to_is_not() {
    // 68:43 and 68:44 read, a stop, and back to read 68:43 again: its 5th-8th words left out there.
    let mut r = Reader::new();
    r.read((68, 43, 1), 21);
    r.hear((68, 44, 10), 3, "Paused");
    r.t += 2.0;
    r.read((68, 43, 1), 4);
    r.read((68, 43, 9), 3);
    assert_eq!(r.marked(), span((68, 43, 5), (68, 43, 8)));
    // Back a few words to say them again (read on three), then on from where the reading had got to, two words short of
    // it, or a word past it: nothing.
    for to in [(68, 44, 2), (68, 44, 4), (68, 44, 5)] {
        let mut r = Reader::new();
        r.read((68, 43, 1), 15);
        r.read((68, 43, 5), 4);
        r.read(to, 3);
        assert!(r.s.is_empty(), "on at {to:?}: {:?}", r.marked());
    }
    // The listener's own step back, two reports, and back with the reader: nothing.
    let mut r = Reader::new();
    r.read((68, 43, 1), 10);
    r.hear((68, 43, 3), 0, "Locked");
    r.hear((68, 43, 3), 2, "Locked");
    r.read((68, 43, 11), 3);
    assert!(r.s.is_empty(), "{:?}", r.marked());
}

#[test]
fn words_a_report_the_hold_kept_back_was_on_are_not_skipped() {
    // The reader stops at 68:43:6, the listener putting them two words back (followed); going on, it hears them at the
    // 9th (held: far ahead of the pointer), at the 4th again (the hold drops them), then at the 10th (held, then
    // followed): the 7th-9th were heard. Not heard there (the 4th again instead), they were left out.
    for heard_there in [true, false] {
        let mut t = 0.0;
        let mut news = reads((68, 43, 1), 6, &mut t);
        news.extend([((68, 43, 4), 0, "Locked", t + 0.32), ((68, 43, 4), 2, "Paused", t + 0.64)]);
        t += 5.64;
        let ninth = if heard_there { (68, 43, 9) } else { (68, 43, 4) };
        news.extend([(ninth, 0, "Locked", t + 0.32), (ninth, 2, "Locked", t + 0.64), ((68, 43, 4), 2, "Locked", t + 0.96)]);
        t += 0.96;
        news.extend(reads((68, 43, 10), 2, &mut t));
        let mut m = marker(&news);
        let want = if heard_there { Vec::new() } else { span((68, 43, 7), (68, 43, 9)) };
        assert_eq!(m.skipped().marks.keys().copied().collect::<Vec<_>>(), want, "heard there {heard_there}");
    }
}

/// The user's reading of page 7 as the listener heard it (its log, 22 Nov): 2:42 to its end and a stop; 2:43:1 heard
/// as the reader went back to 2:40 (its look-alike 2:47 weighed first) and read it again; then 2:42, 2:41 left out (the
/// listener on 2:41:1 for a second, then at 2:42:3, held, with a plan for there before the hold let it go).
const PAGE_7: &str = include_str!("../../../tests/fixtures/listen-page7-skip.jsonl");

#[test]
fn an_ayah_skipped_reading_again_is_marked_from_the_users_log() {
    use super::super::link::{self, Event};
    // As the user has it (tajweed timing: plans), and without.
    for plans in [true, false] {
        let mut settings = Settings::default();
        settings.motion.follow_tajweed = plans;
        let params = Params::of(&settings);
        let mut m = Marker::default();
        let lines: Vec<&str> = PAGE_7.lines().collect();
        for (k, line) in lines.iter().enumerate() {
            match link::parse(line) {
                Some(Event::Heard(h)) => {
                    let t = f64::from(h.t);
                    m.heard(h, t, &params);
                }
                Some(Event::Replan(plan, t)) => m.replan(&plan, t, f64::from(t), &params),
                _ => panic!("not a report: {line}"),
            }
            // Nothing until the jump is followed (the hold lets it go on the last report).
            assert!(k + 1 == lines.len() || m.skipped().is_empty(), "plans {plans}, after {line}: {:?}", m.skipped().summary());
        }
        assert_eq!(m.skipped().summary(), Some(Summary { from: (2, 41, 2), to: (2, 42, 2), words: 19, others: 0 }), "plans {plans}");
        assert_eq!(m.skipped().back_to(), Some(WordRef { chapter: 2, verse: 41, word: 1 }));
    }
}
