//! The script's tests: every scene the same each time and on the listener's grid, both listeners' cadences, each
//! scene's case, where each setting goes, and the passage.

use quran_core::glyph::glyph_code;
use quran_core::reference::{verses_in, words_in};
use quran_core::settings::{MushafFont, Settings};

use super::*;

const BOTH: [Cadence; 2] = [Cadence::Even, Cadence::Heard];

/// A message's place in words into the passage (`None` off it), as the marker reads it but not held short of
/// the word's end.
fn pos(h: &Heard) -> Option<f32> {
    (h.loc.0 == 1).then(|| (h.word - FIRST) as f32 + (f32::from(h.letter) + 0.5) / f32::from(h.letters))
}

/// The messages heard in stretch `i`.
fn of(all: &[Update], i: usize) -> Vec<&Update> {
    all.iter().filter(|u| u.stretch == i).collect()
}

/// The first stretch of `script` there for `moment`.
fn moment(script: &Script, m: Moment) -> usize {
    script.stretches().iter().position(|s| s.moment == m).unwrap()
}

fn with(change: impl Fn(&mut Voice)) -> Voice {
    let mut voice = Voice::default();
    change(&mut voice);
    voice
}

/// The line a word position is on.
fn line(p: f32) -> usize {
    let mut words = 0.0;
    LINES.iter().position(|l| {
        words += l.iter().filter(|g| g.letters > 0).count() as f32;
        p < words
    })
    .unwrap()
}

/// Seconds without a message from time `from` on (until the next one).
fn silent_from(all: &[Update], from: f64) -> f64 {
    all.iter().find(|u| u.at > from + 0.05).map_or(f64::INFINITY, |u| u.at - from)
}

#[test]
fn every_scene_is_the_same_every_time_and_on_the_listeners_grid() {
    for scene in Scene::ALL {
        let script = Script::of(scene, &Voice::default());
        for cadence in BOTH {
            let all = script.updates(cadence);
            assert_eq!(all, script.updates(cadence), "{scene:?} {cadence:?}");
            let most = if cadence == Cadence::Even { 1e-9 } else { 0.035 + 1e-9 };
            for u in &all {
                let tick = (u.at / TICK + 1e-9).floor() * TICK;
                assert!(u.at - tick >= -1e-9 && u.at - tick <= most, "{scene:?} {cadence:?}: {} s after the tick at {tick}", u.at - tick);
                assert!((f64::from(u.heard.t) - tick).abs() < 1e-4, "{scene:?} {cadence:?}: decided at its tick");
                assert!(u.at < script.len(), "{scene:?} {cadence:?}: within the scene");
            }
            assert!(all.windows(2).all(|w| w[0].at < w[1].at), "{scene:?} {cadence:?}: in order");
        }
    }
}

#[test]
fn a_perfect_listener_steps_evenly_and_the_real_one_in_bursts() {
    let reading = |s: &Stretch| matches!(s.ear, Ear::Reading { .. });
    for scene in Scene::ALL {
        let script = Script::of(scene, &Voice::default());
        let even = script.updates(Cadence::Even);
        for (i, s) in script.stretches().iter().enumerate().filter(|(_, s)| reading(s)) {
            // A message on every tick (but the first, where the reader goes on from a letter held).
            let heard = of(&even, i);
            let ticks: Vec<u32> = heard.iter().map(|u| (f64::from(u.heard.t) / TICK).round() as u32).collect();
            let held = i > 0 && script.stretches()[i - 1].truth.is_some_and(|(a, b)| a == b && Some(a) == s.truth.map(|t| t.0));
            assert_eq!(ticks, (s.from + u32::from(held)..s.to).collect::<Vec<_>>(), "{scene:?} stretch {i}");
            let steps: Vec<f32> = heard.windows(2).map(|w| pos(&w[1].heard).unwrap() - pos(&w[0].heard).unwrap()).collect();
            assert!(steps.windows(2).all(|s| (s[0] - s[1]).abs() < 1e-3), "{scene:?} stretch {i}: {steps:?}");
        }
        // Both end each reading stretch within a letter of the reader; every pause is heard a letter back.
        let real = script.updates(Cadence::Heard);
        for cadence in BOTH {
            let all = script.updates(cadence);
            for i in (0..script.stretches().len()).filter(|i| reading(&script.stretches()[*i])) {
                let last = of(&all, i).pop().unwrap();
                let truth = script.truth((f64::from(last.heard.t) * 1e3).round() / 1e3).unwrap();
                assert!((pos(&last.heard).unwrap() - truth).abs() <= 1.0 / f32::from(last.heard.letters) + 1e-4, "{scene:?} {cadence:?} stretch {i}");
            }
        }
        for (i, s) in script.stretches().iter().enumerate().filter(|(_, s)| s.ear == PAUSE) {
            let paused = real.iter().find(|u| u.stretch == i && u.heard.state == "Paused").unwrap();
            assert!(pos(&paused.heard).unwrap() < s.truth.unwrap().0, "{scene:?} stretch {i}");
        }
    }
    // Read's breath, as heard: a step back from the last letter heard.
    let read = Script::of(Scene::Read, &Voice::default());
    let real = read.updates(Cadence::Heard);
    let paused = real.iter().position(|u| u.heard.state == "Paused").unwrap();
    assert!(pos(&real[paused].heard) < pos(&real[paused - 1].heard));
    // Reading on in Read: one stretch of 6.4 s or more over two lines, which the real listener hears with all of
    // its pattern: ticks without news, falling behind, then most of a word at once, at about the reader's pace.
    let (i, long) = read.stretches().iter().enumerate().max_by_key(|(_, s)| s.to - s.from).unwrap();
    assert!(long.until() - long.start() >= 6.4 && long.moment == Moment::Plain);
    let (a, b) = long.truth.unwrap();
    assert!(line(a) != line(b - 0.01), "over a line's end");
    let heard = of(&real, i);
    let ticks: Vec<f64> = heard.iter().map(|u| f64::from(u.heard.t)).collect();
    assert!(ticks.windows(2).filter(|w| w[1] - w[0] > 1.5 * TICK).count() >= 2, "silent ticks: {ticks:?}");
    let steps: Vec<f32> = heard.windows(2).map(|w| pos(&w[1].heard).unwrap() - pos(&w[0].heard).unwrap()).collect();
    assert!(steps.iter().any(|s| *s > 2.5 * PACE * TICK as f32), "a burst: {steps:?}");
    let behind = heard.iter().map(|u| read.truth(f64::from(u.heard.t)).unwrap() - pos(&u.heard).unwrap()).fold(0.0, f32::max);
    assert!(behind > 0.6, "falls behind: {behind}");
    let (first, last) = (heard[0], heard[heard.len() - 1]);
    let pace = (pos(&last.heard).unwrap() - pos(&first.heard).unwrap()) / (last.heard.t - first.heard.t);
    assert!((pace / PACE - 1.0).abs() < 0.1, "pace {pace}");
}

#[test]
fn each_scene_holds_its_case() {
    for cadence in BOTH {
        let read = Script::of(Scene::Read, &Voice::default());
        let all = read.updates(cadence);
        assert_eq!((all[0].heard.state.as_str(), all[1].heard.state.as_str(), all[2].heard.state.as_str()), ("Tentative", "Tentative", "Locked"));
        // The ayah's end: paused once in 1:6's last word, for as long as the fade's delay needs.
        for (delay, off) in [(0, false), (1000, false), (4000, false), (10_000, false), (1000, true)] {
            let script = Script::of(Scene::AyahEnd, &with(|v| (v.motion.ayah_fade_delay_ms, v.motion.fade_at_ayah_end) = (delay, !off)));
            let all = script.updates(cadence);
            let i = moment(&script, Moment::AyahEnd);
            let s = &script.stretches()[i];
            let pause = s.until() - s.start();
            assert!(pause >= f64::from(delay) / 1000.0 + 1.2 && pause >= 1.9, "{cadence:?} {delay}: {pause}");
            let paused: Vec<&Update> = of(&all, i).into_iter().filter(|u| u.heard.state == "Paused").collect();
            assert_eq!((paused.len(), paused[0].heard.loc), (1, (1, 6, 3)), "{cadence:?}");
        }
        // A letter held 2.5 s and more without a message.
        let hold = Script::of(Scene::Hold, &Voice::default());
        let start = hold.stretches()[moment(&hold, Moment::Held)].start();
        assert!(silent_from(&hold.updates(cadence), start) >= 2.5, "{cadence:?}");
        // The slips: a word back, Locked, held for as long as the wait needs; a skip ahead; back over the line.
        for wait in [0, 1500, 5000] {
            let script = Script::of(Scene::Slips, &with(|v| v.motion.wait_ms = wait));
            let all = script.updates(cadence);
            let step = |i: usize| {
                let first = all.iter().position(|u| u.stretch == i).unwrap();
                (pos(&all[first].heard).unwrap() - pos(&all[first - 1].heard).unwrap(), &all[first])
            };
            let (repeat, heard) = step(moment(&script, Moment::Repeat));
            assert!(repeat < -0.75 && repeat > -0.9 && heard.heard.state == "Locked", "{cadence:?}: {repeat}");
            assert!(silent_from(&all, heard.at) >= f64::from(wait) / 1000.0 + 1.2, "{cadence:?} {wait}: held");
            assert!(step(moment(&script, Moment::Skip)).0 > 3.5, "{cadence:?}");
            let (back, heard) = step(moment(&script, Moment::Back));
            let first = pos(&heard.heard).unwrap();
            assert!(back < -3.0 && line(first) != line(first - back), "{cadence:?}: {back}");
        }
        // A far place heard for less than 1.5 s, a word on, after 2 s and more read on the passage.
        let misheard = Script::of(Scene::Misheard, &Voice::default());
        let all = misheard.updates(cadence);
        let i = moment(&misheard, Moment::Misheard);
        let far = of(&all, i);
        let (a, b) = (far[0], far[far.len() - 1]);
        assert!(b.at - a.at < 1.5 && b.heard.word - a.heard.word < 3 && a.heard.loc.0 != 1, "{cadence:?}");
        assert!(misheard.stretches()[i].start() >= 2.5 && all.last().unwrap().heard.loc.0 == 1);
        // Then one read on for good, as far as it goes.
        let away = Script::of(Scene::Far, &Voice::default());
        let i = moment(&away, Moment::Away);
        assert!(away.len().is_infinite() && away.moved() == Some(away.stretches()[i].start()));
        let all = away.updates(cadence);
        let far = of(&all, i);
        assert_eq!(far.last().unwrap().heard.word - far[0].heard.word, 14);
    }
}

#[test]
fn a_listener_that_plans_sends_its_plans_and_the_voice_level_when_asked() {
    let off = Script::of(Scene::Hold, &Voice::default());
    assert!(off.updates(Cadence::Heard).iter().all(|u| u.heard.plan.is_none()) && off.voices().is_empty(), "as before");
    let on = Script::of(Scene::Hold, &with(|v| v.motion.follow_tajweed = true));
    for cadence in BOTH {
        let all = on.updates(cadence);
        assert!(all.iter().all(|u| u.heard.plan.is_some() == (u.heard.state == "Locked")), "{cadence:?}");
        // The letter held: a plan that creeps across it in the hold's time, its seq grown with the place heard.
        let held = of(&all, moment(&on, Moment::Held))[0].heard.plan.clone().unwrap();
        assert!(held.hold.is_some_and(|(a, b)| a == 0.0 && b > 2.0) && held.knots.len() == 1 && held.now + held.knots[0].1 <= held.ev_end + 1e-4, "{held:?}");
        let seqs: Vec<u32> = all.iter().filter_map(|u| u.heard.plan.as_ref().map(|p| p.seq)).collect();
        assert!(seqs.windows(2).all(|w| w[1] >= w[0]) && seqs.last() > seqs.first(), "{seqs:?}");
    }
    // The voice: present while reading and holding, a silence through the breath inside the ayah.
    let voices = on.voices();
    let at = |t: f64| voices.iter().rfind(|v| v.0 <= t).unwrap().1;
    let quiet = on.stretches()[moment(&on, Moment::Quiet)].start();
    assert!(at(quiet - 0.1).present && !at(quiet + 0.5).present && at(quiet + 0.5).quiet, "{voices:?}");
    assert!(at(on.stretches()[moment(&on, Moment::Held)].start() + 1.0).present);
}

#[test]
fn each_motion_setting_goes_to_its_scene_and_the_looks_nowhere() {
    let base = Voice::default();
    let scene = |change: fn(&mut Voice)| {
        let mut new = base.clone();
        change(&mut new);
        assert_ne!(new, base);
        scene_for(&base, &new)
    };
    assert_eq!(scene(|v| v.motion.max_ahead = 12), Some(Scene::Hold));
    assert_eq!(scene(|v| v.motion.wait_ms = 0), Some(Scene::Slips));
    assert_eq!(scene(|v| v.motion.fade_back = 5), Some(Scene::Slips));
    assert_eq!(scene(|v| v.motion.far_jump = 3), Some(Scene::Slips));
    assert_eq!(scene(|v| v.motion.fade_at_ayah_end = false), Some(Scene::AyahEnd));
    assert_eq!(scene(|v| v.motion.ayah_fade_delay_ms = 3000), Some(Scene::AyahEnd));
    assert_eq!(scene(|v| v.motion.follow_tajweed = true), Some(Scene::Hold));
    assert_eq!(scene(|v| v.motion.quiet_dim = 35), Some(Scene::Hold));
    assert_eq!(scene(|v| v.motion.hold_when_quiet = true), Some(Scene::Hold));
    assert_eq!(scene(|v| v.far_wait_ms = 0), Some(Scene::Far));
    assert_eq!(scene(|v| v.auto_scroll = false), Some(Scene::Far));
    for look in [
        |v: &mut Voice| v.pointer.size = 12,
        |v: &mut Voice| v.pointer.smudge = 8,
        |v: &mut Voice| v.ring = 3,
        |v: &mut Voice| v.lift = 5,
        |v: &mut Voice| v.range.blur = 6,
        |v: &mut Voice| v.range.border = 2,
        |v: &mut Voice| v.motion.pointer.curve = quran_core::settings::VoiceCurve::Spring,
        |v: &mut Voice| v.motion.range_start.ms = 900,
        |v: &mut Voice| v.motion.range_end.ms = 100,
    ] {
        assert_eq!(scene(look), None);
    }
}

/// The index in the whole Quran of `s:a:w`, from the reference tables.
fn index(surah: u16, ayah: u16, word: u16) -> u32 {
    let whole = (1..surah).flat_map(|s| (1..=verses_in(s).unwrap()).map(move |a| (s, a)));
    let before: usize = whole.chain((1..ayah).map(|a| (surah, a))).map(|(s, a)| words_in(s, a).unwrap()).sum();
    (before + usize::from(word) - 1) as u32
}

#[test]
fn the_passage_is_page_ones_glyphs_from_1_6_on() {
    let codes: Vec<char> = LINES.iter().flat_map(|l| l.iter()).map(|g| g.code).collect();
    assert_eq!(codes, ('\u{FC57}'..='\u{FC64}').collect::<Vec<_>>());
    let ends: Vec<char> = LINES.iter().flat_map(|l| l.iter()).filter(|g| g.letters == 0).map(|g| g.code).collect();
    assert_eq!(ends, ['\u{FC5A}', '\u{FC64}']);
    assert_eq!(words().map(|g| g.loc).collect::<Vec<_>>(), (1..=3).map(|w| (1, 6, w)).chain((1..=9).map(|w| (1, 7, w))).collect::<Vec<_>>());
    assert_eq!((FIRST, MISHEARD.word, ELSEWHERE.word), (index(1, 6, 1), index(4, 69, 9), index(2, 255, 1)));
}

#[test]
#[ignore = "asks api.quran.com"]
fn live_the_passage_is_the_apis_in_both_fonts() {
    let client = quran_core::api::Client::default();
    for font in [MushafFont::TajweedV4, MushafFont::MadaniV2] {
        let query = Settings { mushaf_font: font, ..Settings::default() }.verse_query();
        let page = client.verses_by_page(1, &query).expect("page 1");
        let words: Vec<_> = page.verses.iter().filter(|v| v.verse_key == "1:6" || v.verse_key == "1:7").flat_map(|v| &v.words).collect();
        let ours: Vec<&Glyph> = LINES.iter().flat_map(|l| l.iter()).collect();
        assert_eq!(words.len(), ours.len(), "{font:?}");
        for (word, glyph) in words.iter().zip(ours) {
            assert_eq!(glyph_code(word).and_then(|c| c.chars().next()), Some(glyph.code), "{font:?} {:?}", glyph.loc);
            if !word.is_end_marker() {
                assert_eq!(word.text_uthmani.as_deref(), Some(glyph.text), "{font:?} {:?}", glyph.loc);
            }
        }
    }
}
