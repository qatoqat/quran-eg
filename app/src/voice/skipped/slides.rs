//! Slides into look-alike ayahs, through the marker and its frames over a page of the mushaf (turned where the pointer
//! steers, as Page view does).

use egui::{LayerId, Rect, pos2, vec2};
use quran_core::reference::{page_of, verses_on_page};
use quran_core::settings::Voice as Settings;

use super::super::follow::Offer;
use super::super::link::{self, Event};
use super::super::marker::{Marker, Stage};
use super::super::motion::Params;
use super::tests::{index, report};
use super::*;

/// The words of mushaf page `n` (a surah's Bismillah line's places before its first ayah), eight to a line.
fn page(n: u16) -> Vec<Slot> {
    let mut locs = Vec::new();
    for (s, span) in verses_on_page(n) {
        for a in span.from..=span.to {
            if a == 1 && s != 1 && s != 9 {
                locs.extend((1..=4).map(|w| (s, 0, w)));
            }
            locs.extend((1..=words_in(s, a).unwrap() as u16).map(|w| (s, a, w)));
        }
    }
    (0..)
        .zip(locs)
        .map(|(k, loc): (u16, Loc)| {
            let rect = Rect::from_min_size(pos2(900.0 - 100.0 * f32::from(k % 8), 60.0 + 45.0 * f32::from(k / 8)), vec2(90.0, 40.0));
            (loc, rect, rect.top() + 30.0, 28.0)
        })
        .collect()
}

/// A frame: its clock time, the words a slide marked, the slide's toast and the far place offered.
struct Frame {
    t: f64,
    similar: Vec<Loc>,
    slide: Option<Slide>,
    offer: Option<Offer>,
}

/// The marker hearing `news` (clock time, event) over page `start`, a frame every 0.1 s until a second after the last.
fn run(news: &[(f64, Event)], settings: &Settings, start: u16) -> Vec<Frame> {
    let (params, ctx, mut m) = (Params::of(settings), egui::Context::default(), Marker::default());
    let colors = crate::theme::Colors::of(&quran_core::settings::Settings::default(), false);
    let (mut words, mut out, mut k) = (page(start), Vec::new(), 0);
    let mut now = news[0].0;
    while now < news.last().unwrap().0 + 1.0 {
        while k < news.len() && news[k].0 <= now + 1e-9 {
            match &news[k].1 {
                Event::Heard(h) => drop(m.heard(h.clone(), news[k].0, &params)),
                Event::Replan(plan, t) => m.replan(plan, *t, news[k].0, &params),
                _ => {}
            }
            k += 1;
        }
        let mut shown = None;
        let mut output = ctx.run_pass(egui::RawInput { time: Some(now), ..Default::default() }, |ctx| {
            let layer = |n| LayerId::new(egui::Order::Middle, egui::Id::unique(n));
            let stage = Stage { now, dt: 0.1, area: Rect::from_min_size(pos2(0.0, 0.0), vec2(1000.0, 1100.0)), layers: (layer("r"), layer("p")), scale: 1.0, scrolls: false };
            shown = Some(m.frame(ctx, &stage, settings, &colors, &words));
        });
        output.textures_delta.clear();
        let shown = shown.unwrap();
        if let Some(to) = shown.steer.go_to.and_then(|w| page_of(w.chapter, w.verse)) {
            words = page(to);
        }
        let s = m.skipped();
        out.push(Frame { t: now, similar: s.words(Kind::Similar), slide: s.slide(), offer: shown.offer });
        now += 0.1;
    }
    out
}

/// The listener's reports and plans in `text` (lines of its log), each taken a little after it was decided.
fn logged(text: &str) -> Vec<(f64, Event)> {
    let at = |t: f32| f64::from(t) + 0.08;
    text.lines()
        .map(|line| match link::parse(line) {
            Some(Event::Heard(h)) => (at(h.t), Event::Heard(h)),
            Some(Event::Replan(plan, t)) => (at(t), Event::Replan(plan, t)),
            _ => panic!("not a report: {line}"),
        })
        .collect()
}

/// Two reports a word (Locked, sure), 0.32 s apart, from clock time `t` on, `words` words from `loc`.
fn read(news: &mut Vec<(f64, Event)>, t: &mut f64, loc: Loc, words: i64) {
    for k in 0..words {
        for letter in [0, 2] {
            *t += 0.32;
            let h = Heard { t: *t as f32, ..report(step(loc, k).unwrap(), letter, "Locked") };
            news.push((*t, Event::Heard(h)));
        }
    }
}

/// The user's reading of page 10 as the listener heard it (its log, 22 Nov): 2:62's end, 2:63 from its start, and after
/// its third word (وَإِذْ أَخَذْنَا مِيثَاقَكُمْ, which 2:84 shares) 2:84's fourth and fifth; a stop, 2:84's shared words
/// said again a few times, then back to 2:63:4 and on.
const PAGE_10: &str = include_str!("../../../tests/fixtures/listen-page10-similar.jsonl");

#[test]
fn a_slide_into_a_far_look_alike_marks_where_the_ayah_goes_on_until_it_is_read_from_the_users_log() {
    let (a4, a5, a6) = ((2, 63, 4), (2, 63, 5), (2, 63, 6));
    // As the user has it (tajweed timing: plans), and without.
    for plans in [true, false] {
        let mut settings = Settings::default();
        settings.motion.follow_tajweed = plans;
        let frames = run(&logged(PAGE_10), &settings, 10);
        let first = frames.iter().position(|f| !f.similar.is_empty()).unwrap();
        // Heard in 2:84 from 124.16 s (taken at 124.24): marked once heard there a second, as a far look-alike is offered,
        // three words from where the two part; the toast names the look-alike. No offer is made for it.
        assert!((125.2..125.5).contains(&frames[first].t), "plans {plans}: marked at {:.2}", frames[first].t);
        assert_eq!(frames[first].similar, [a4, a5, a6], "plans {plans}");
        assert_eq!(frames[first].slide, Some(Slide { from: a4, into: (2, 84, 5) }));
        assert!(frames.iter().all(|f| f.offer.is_none()), "plans {plans}: no offer");
        // Back at 2:63:4 (136.0 s), the words read lose their marks, one by one, by 2:63:6 (137.92 s); the look-alike heard
        // once more as the reader comes back marks nothing again.
        let at = |t: f64| &frames.iter().rfind(|f| f.t <= t).unwrap().similar;
        assert_eq!((at(135.9), at(136.5), at(137.9)), (&vec![a4, a5, a6], &vec![a5, a6], &vec![a6]), "plans {plans}");
        assert!(frames.iter().filter(|f| f.t > 138.1).all(|f| f.similar.is_empty() && f.slide.is_none()), "plans {plans}");
    }
}

/// The user's reading of 2:68 a little later (its log, 22 Nov): after 2:68:13 (بَقَرَةٌ, which 2:69 shares with it from
/// قَالَ إِنَّهُ يَقُولُ إِنَّهَا), 2:69:14-16 (صَفْرَاءُ فَاقِعٌ لَوْنُهَا) on the same page; then back to 2:68:15 and on.
const PAGE_10_NEAR: &str = include_str!("../../../tests/fixtures/listen-page10-similar-near.jsonl");

#[test]
fn a_slide_into_a_look_alike_on_the_page_is_marked_as_one_not_as_a_skip_from_the_users_log() {
    for plans in [true, false] {
        let mut settings = Settings::default();
        settings.motion.follow_tajweed = plans;
        let params = Params::of(&settings);
        let mut m = Marker::default();
        for (now, e) in logged(PAGE_10_NEAR) {
            match e {
                Event::Heard(h) => drop(m.heard(h, now, &params)),
                Event::Replan(plan, t) => m.replan(&plan, t, now, &params),
                _ => {}
            }
            let s = m.skipped();
            // The jump on is followed once the hold lets it go (111.68 s, the listener saying it slid): never a skip.
            assert!(s.summary().is_none(), "plans {plans}, at {now:.2}: {:?}", s.summary());
            let want: &[Loc] = match now {
                ..111.7 => &[],
                ..114.6 => &[(2, 68, 14), (2, 68, 15), (2, 68, 16)],
                ..115.5 => &[(2, 68, 16)],
                _ => &[],
            };
            assert_eq!(s.words(Kind::Similar), want, "plans {plans}, at {now:.2}");
            if !want.is_empty() {
                assert_eq!(s.slide().map(|s| s.into), Some((2, 69, 14)));
            }
        }
    }
}

#[test]
fn with_the_marks_off_a_far_look_alike_is_offered_as_before() {
    let mut settings = Settings::default();
    settings.similar.enabled = false;
    let frames = run(&logged(PAGE_10), &settings, 10);
    assert!(frames.iter().all(|f| f.similar.is_empty()));
    let offered = frames.iter().find_map(|f| f.offer).unwrap();
    assert_eq!(offered, Offer { to: (2, 84, 5), left: None });
}

#[test]
fn a_brief_excursion_into_a_far_look_alike_marks_nothing() {
    // 2:63 read from its start, the place followed for a while; then 2:84:4-5 heard for two reports (0.64 s), and back
    // with 2:63:4: the listener's own mishearing, or a slip caught at once.
    let (mut news, mut t) = (Vec::new(), 100.0);
    read(&mut news, &mut t, (2, 62, 20), 5);
    read(&mut news, &mut t, (2, 63, 1), 3);
    read(&mut news, &mut t, (2, 84, 4), 1);
    read(&mut news, &mut t, (2, 63, 4), 4);
    let frames = run(&news, &Settings::default(), 10);
    assert!(frames.iter().all(|f| f.similar.is_empty() && f.offer.is_none()));
    // Kept there, read on (2:84:4-6 over 1.9 s): a slide.
    let (mut news, mut t) = (Vec::new(), 100.0);
    read(&mut news, &mut t, (2, 62, 20), 5);
    read(&mut news, &mut t, (2, 63, 1), 3);
    read(&mut news, &mut t, (2, 84, 4), 3);
    let frames = run(&news, &Settings::default(), 10);
    assert_eq!(frames.last().unwrap().similar, [(2, 63, 4), (2, 63, 5), (2, 63, 6)]);
}

#[test]
fn a_far_look_alike_near_where_the_reading_jumped_from_is_the_listener_coming_back() {
    // 56:13 and 56:39 are one ayah (ثُلَّةٌ مِنَ الْأَوَّلِينَ), 56:14 and 56:40 share their last words. Reading 56:11-14 on
    // page 534, the listener goes on to 56:39-41 on page 535 (followed there), then comes back to 56:14 as it slides
    // (from 56:39:3): back where the reader was, not a slide. Read there in order from 56:35, the same slide is one,
    // marked where 56:39 goes on (56:40, all three words).
    let slid = |news: &mut [(f64, Event)]| {
        for (_, e) in news {
            if let Event::Heard(h) = e {
                h.slid = Some(((56, 39, 3), (56, 14, 1)));
            }
        }
    };
    let (mut news, mut t) = (Vec::new(), 100.0);
    read(&mut news, &mut t, (56, 11, 1), 11);
    read(&mut news, &mut t, (56, 39, 1), 10);
    let back = news.len();
    read(&mut news, &mut t, (56, 14, 1), 9);
    slid(&mut news[back..]);
    let frames = run(&news, &Settings::default(), 534);
    assert!(frames.iter().all(|f| f.similar.is_empty()));
    let offers: Vec<Offer> = frames.iter().filter_map(|f| f.offer).collect();
    assert!(!offers.is_empty() && offers.iter().all(|o| o.left.is_none() && (14..=16).contains(&o.to.1)), "{offers:?}");
    let (mut news, mut t) = (Vec::new(), 100.0);
    read(&mut news, &mut t, (56, 35, 1), 12);
    let back = news.len();
    read(&mut news, &mut t, (56, 14, 1), 4);
    slid(&mut news[back..]);
    let frames = run(&news, &Settings::default(), 535);
    assert_eq!(frames.last().unwrap().similar, [(56, 40, 1), (56, 40, 2), (56, 40, 3)]);
    assert!(frames.iter().all(|f| f.offer.is_none()));
}

#[test]
fn a_slide_marks_three_words_or_the_rest_of_the_ayah_and_reading_them_from_anywhere_clears_them() {
    // 2:63 has 15 words.
    let marked = |from: Loc| {
        let mut s = Skipped::default();
        s.slid((from, index(from)), (2, 84, 5));
        s.words(Kind::Similar)
    };
    assert_eq!(marked((2, 63, 4)), [(2, 63, 4), (2, 63, 5), (2, 63, 6)]);
    assert_eq!(marked((2, 63, 10)).len(), 3);
    assert_eq!(marked((2, 63, 11)), span((2, 63, 11), (2, 63, 15)));
    assert_eq!(marked((2, 63, 15)), [(2, 63, 15)]);
    // Back from the look-alike a word in, whether it lies after the ayah (2:84) or before it (2:40): that word and the
    // one before lose their marks; the next word read, its own.
    for far in [(2, 84, 6), (2, 40, 6)] {
        let mut s = Skipped::default();
        s.slid(((2, 63, 4), index((2, 63, 4))), (2, 84, 5));
        let hear = |s: &mut Skipped, loc: Loc, t: f64| s.heard(&report(loc, 1, "Locked"), Reading::Locked, None, 1.5, Some(2), t);
        hear(&mut s, far, 1.0);
        hear(&mut s, (2, 63, 5), 1.32);
        assert_eq!(s.words(Kind::Similar), [(2, 63, 6)], "from {far:?}");
        assert_eq!(s.slide(), Some(Slide { from: (2, 63, 6), into: (2, 84, 5) }));
        hear(&mut s, (2, 63, 6), 1.64);
        assert!(s.slide().is_none() && s.is_empty(), "from {far:?}");
    }
    // Dismissed, the skipped words stay; stopped, all go.
    let mut s = Skipped::default();
    s.slid(((2, 63, 4), index((2, 63, 4))), (2, 84, 5));
    s.marks.insert((68, 45, 1), index((68, 45, 1)));
    s.dismiss(Kind::Similar);
    assert!(s.slide().is_none() && s.summary().is_some());
    s.slid(((2, 63, 4), index((2, 63, 4))), (2, 84, 5));
    s.clear();
    assert!(s.is_empty());
}

/// The words from `from` to `to`.
fn span(from: Loc, to: Loc) -> Vec<Loc> {
    let mut out = vec![from];
    while *out.last().unwrap() != to {
        out.push(step(*out.last().unwrap(), 1).unwrap());
    }
    out
}
