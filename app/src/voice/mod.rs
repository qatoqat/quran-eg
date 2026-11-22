//! The voice pointer (native builds): quran-voice's listener hears the recitation and a dot on the
//! letter being read moves across the mushaf, inside an accuracy range (a rounded rectangle over the
//! words it could be on), as the quran.com extension does. `link` talks to the listener; `marker` draws
//! the dot and the range for what it heard; `motion` smooths its bursty updates into a glide at the
//! reader's pace, each part of the pointer easing as `glide` does; `follow` decides when the reader goes
//! to a place heard off screen, and `lead` when a basmala opens the next surah; `visible` tells the
//! listener which words are on screen and whether the reader chose them; `meter` shows on the Listen
//! button how the microphone hears, and `presence` whether the reader's voice is heard (from the
//! levels the listener sends when the settings ask). `preview` shows in
//! the settings `script`, a recitation of a few lines with a listener of its own, which `player` plays
//! through the same marker (`dock` draws the rest), so their changes can be seen at work.
//! `installed` tells there which listener is installed and which models it can run. Tuned movement: `hold` keeps a jump
//! ahead back until the reading goes on there; `keep` keeps the line being read in view; `skipped` marks the words the
//! reading went past unread until they are read; `toast` says where the reader is about to be taken, which words they
//! skipped, and, with `weak`, when the microphone hears them too faintly. `replay` plays the newest
//! recording through `sound` while a listener of its own hears it, so the pointer follows it as it does the reader.

mod dock;
mod follow;
mod glide;
mod hold;
mod installed;
mod keep;
mod lead;
mod link;
mod marker;
mod meter;
mod motion;
mod player;
mod presence;
mod preview;
mod replay;
mod script;
mod skipped;
mod sound;
mod toast;
mod visible;
mod weak;

use egui::Rect;
use quran_core::api::Chapter;
use quran_core::settings::Voice as Settings;
use quran_core::view::WordRef;

use self::link::{Event, Heard, Link, Want};
use self::marker::{Marker, Stage, Wake, frame_dt};
use self::meter::Meter;
use self::toast::{Click, Note, Toast};
use self::weak::Weak;
use self::motion::Params;
use self::visible::Visible;
use crate::header::Listening;
use crate::kit::Mic;
use crate::pointer::Lens;
use crate::theme::Colors;

pub use self::glide::CURVES;
pub use self::installed::{Installed, Model};
pub use self::link::folder;
pub use self::link::Loc;
pub use self::preview::Preview;
pub use self::replay::{Replay, Tick};

use crate::reader::Slot;

/// Seconds the connecting spinner takes to turn once.
const SPIN: f64 = 1.6;

#[derive(Debug, Clone, PartialEq)]
pub enum Status {
    Starting,
    Listening,
    Failed(String),
}

/// What the reader should do for the voice pointer.
#[derive(Debug, Default, PartialEq)]
pub struct Steer {
    /// Scroll by this much, to keep the line being read in view.
    pub scroll: f32,
    /// The place being read is not on screen: go there.
    pub go_to: Option<WordRef>,
    /// A basmala opens this surah, not on screen: go to its start, its banner and Bismillah in view.
    pub lead: Option<u16>,
    /// The reader closed the reading with ṣadaqa-llāhu-l-ʿaẓīm.
    pub closed: bool,
}

pub struct Voice {
    link: Link,
    status: Status,
    marker: Marker,
    /// The words on screen as the listener hears of them, and whether the reader chose them.
    visible: Visible,
    /// The reader has just closed with ṣadaqa-llāhu-l-ʿaẓīm (until the next frame reports it).
    closed: bool,
    meter: Meter,
    /// How far the reader's voice stands over the room's sound (dB), from a listener that sends its level.
    voice_range: Option<f32>,
    /// What the settings last asked the listener for, and what it has sent: a plan, a voice level, and a Locked place
    /// since it was asked (with which a listener that plans sends a plan), and when it was asked.
    want: Option<Want>,
    offers: Offers,
    /// The microphone hearing the voice weakly, for the toast.
    weak: Weak,
    /// The line the dot was last drawn on (its y and the text size), while the view stays: a toast appears under it,
    /// and stays where it appeared.
    line: Option<(f32, f32)>,
    toast: toast::Pin,
    /// The model it was started with (a [`Settings::model`]).
    model: String,
    /// It follows a replay of a recording, not the microphone.
    replay: bool,
}

/// What the joined listener has sent beside its pointers.
#[derive(Default)]
struct Offers {
    plans: bool,
    voice: bool,
    locked: bool,
    since: f64,
}

/// The listener's formulas outside the mushaf (surah 0), by number, for the title bar.
fn formula(ayah: u16) -> &'static str {
    match ayah {
        1 => "isti'adha",
        2 => "basmala",
        3 | 4 => "sadaqa-llahu-l-'azim",
        _ => "takbir",
    }
}

/// The last word of ṣadaqa-llāhu-l-ʿaẓīm (0:3) or of its form with al-ʿaliyy (0:4).
fn closes(loc: Loc) -> bool {
    matches!(loc, (0, 3, 3) | (0, 4, 4))
}

/// What the settings want from the listener beside its pointers: plans for tajweed timing (they come with the voice
/// level), the voice level for the hold and the dim.
fn wants(settings: &Settings) -> Want {
    let m = &settings.motion;
    Want { plans: m.follow_tajweed, voice: m.follow_tajweed || m.hold_when_quiet || m.quiet_dim < 100 }
}

/// What is asked of the listener: what the settings want, and the voice level always (the weak microphone's toast).
fn asked(settings: &Settings) -> Want {
    Want { voice: true, ..wants(settings) }
}

/// `s:a:w`.
fn key(loc: Loc) -> String {
    format!("{}:{}:{}", loc.0, loc.1, loc.2)
}

/// Asks for the frame the pointer wants next.
fn ask_frame(ctx: &egui::Context, wake: Wake) {
    match wake {
        Wake::Now => ctx.request_repaint(),
        Wake::After(secs) => ctx.request_repaint_after_secs(secs),
        Wake::Rest => {}
    }
}

impl Voice {
    /// Joins quran-voice's listener, or starts the installed one with the model the settings name.
    pub fn start(settings: &Settings, ctx: &egui::Context) -> Result<Voice, String> {
        let link = Link::start(link::folder(&settings.listener), &settings.model, ctx.clone())?;
        Ok(Voice::with(link, &settings.model, false))
    }

    /// Starts the installed listener on the recording `wav` for a replay, to hear it from when `cue`'s sound plays it.
    fn replay(settings: &Settings, ctx: &egui::Context, wav: &std::path::Path, cue: std::sync::Arc<sound::Cue>) -> Result<Voice, String> {
        let link = Link::replay(link::folder(&settings.listener), &settings.model, wav, cue, ctx.clone())?;
        Ok(Voice::with(link, &settings.model, true))
    }

    fn with(link: Link, model: &str, replay: bool) -> Voice {
        Voice {
            link,
            status: Status::Starting,
            marker: Marker::default(),
            visible: Visible::default(),
            closed: false,
            meter: Meter::default(),
            voice_range: None,
            want: None,
            offers: Offers::default(),
            weak: Weak::default(),
            line: None,
            toast: toast::Pin::default(),
            model: model.to_owned(),
            replay,
        }
    }

    pub fn status(&self) -> &Status {
        &self.status
    }

    /// The model its listener was started with (a [`Settings::model`]).
    pub fn model(&self) -> &str {
        &self.model
    }

    /// It joined a listener started elsewhere, which runs a model of its own choosing.
    pub fn joined(&self) -> bool {
        self.link.joined()
    }

    /// One line for the title bar: what the listener hears (or that the microphone does not).
    pub fn summary(&self) -> String {
        match self.meter.trouble().filter(|_| self.status == Status::Listening) {
            Some(trouble) => format!("◉ Listening · {trouble}"),
            None => self.heard_summary(),
        }
    }

    /// Where the listener hears the reader, once it does: a place, or that it lost it.
    fn place(&self) -> Option<String> {
        if let Some(s) = self.marker.stray() {
            return Some(format!("lost the place ({}:{} heard elsewhere)", s.to.0, s.to.1));
        }
        let h = self.marker.last()?;
        Some(match h.loc {
            (0, ayah, _) => format!("{} · {}", formula(ayah), h.state),
            (surah, 0, _) => format!("basmala of surah {surah} · {}", h.state),
            (surah, ayah, word) => format!("{surah}:{ayah} word {word} · {}", h.state),
        })
    }

    /// What the listener hears, or that it is starting or has stopped.
    fn heard_summary(&self) -> String {
        match &self.status {
            Status::Starting if self.link.started_it() => "◉ Starting the listener…".into(),
            Status::Starting => "◉ Joining the listener…".into(),
            Status::Failed(e) => format!("Listening stopped: {e}"),
            Status::Listening => self.place().map_or_else(|| "◉ Listening: recite from any page".into(), |p| format!("◉ Listening · {p}")),
        }
    }

    /// What a replay's listener hears of the recording, for the title bar.
    pub fn replay_summary(&self) -> String {
        match &self.status {
            Status::Starting => "starting the listener…".into(),
            _ => self.place().unwrap_or_else(|| "listening…".into()),
        }
    }

    /// The Listen button: its microphone, and what its tooltip says.
    pub fn listening(&self, now: f64) -> Listening {
        let (mic, mut mic_line) = match self.status {
            Status::Starting => (Mic::Connecting((now / SPIN).fract() as f32), None),
            Status::Listening => (self.meter.mic(now), self.meter.line(now)),
            Status::Failed(_) => (Mic::Off, None),
        };
        if let Some(r) = self.voice_range.filter(|&r| r < weak::WEAK && self.status == Status::Listening) {
            let hint = format!("Your voice stands only {r:.0} dB over the room's sound: more input gain or a closer microphone helps the pointer");
            mic_line = Some(mic_line.map_or(hint.clone(), |l| format!("{l}\n{hint}")));
        }
        // The microphone's trouble has its own line there.
        Listening { summary: self.heard_summary(), mic, mic_line }
    }

    /// How opaque the page is (fading for a move): the pointer and its range fade with it.
    pub fn set_opacity(&mut self, opacity: f32) {
        self.marker.set_out(1.0 - opacity);
    }

    /// The word last followed on screen.
    pub fn seen(&self) -> Option<quran_core::view::WordRef> {
        self.marker.seen().map(|(chapter, verse, word)| quran_core::view::WordRef { chapter, verse, word })
    }

    /// The glow's lens at the dot, if it glows.
    pub fn lens(&self, settings: &Settings) -> Option<Lens> {
        self.marker.lens(settings)
    }

    /// Why the pointer's settings do not take, if the joined listener has sent none of what they need: an older listener
    /// plans nothing and sends no voice level (Voice › Motion says so).
    pub fn lacks(&self, settings: &Settings, now: f64) -> Option<&'static str> {
        let (want, got) = (wants(settings), &self.offers);
        let joined = self.status == Status::Listening && self.want == Some(asked(settings)) && now - got.since > 2.0;
        if joined && want.plans && got.locked && !got.plans {
            Some("The listener sends no tajweed plan: it is an older one (install quran-voice's listener again). The pointer moves as before.")
        } else if joined && want.voice && !got.voice {
            Some("The listener sends no voice level: it is an older one (install quran-voice's listener again). Nothing holds or dims in silence.")
        } else {
            None
        }
    }

    fn heard(&mut self, h: Heard, now: f64, params: &Params) {
        self.closed |= closes(h.loc) && h.state == "Locked";
        self.offers.plans |= h.plan.is_some();
        self.offers.locked |= h.state == "Locked" && h.loc.0 > 0;
        self.marker.heard(h, now, params);
    }

    /// Reads the listener's news and moves the microphone's meter. Call before the header, which shows
    /// them.
    pub fn poll(&mut self, ctx: &egui::Context, settings: &Settings) {
        let now = ctx.input(|i| i.time);
        let params = Params::of(settings);
        for event in self.link.poll() {
            match event {
                Event::Connected => {
                    self.status = Status::Listening;
                    (self.meter, self.voice_range, self.want, self.offers) = (Meter::default(), None, None, Offers::default());
                }
                Event::Heard(h) => self.heard(h, now, &params),
                Event::Replan(plan, t) => {
                    self.offers.plans = true;
                    self.marker.replan(&plan, t, now, &params);
                }
                Event::Voice { heard, range, loud } => {
                    self.offers.voice = true;
                    self.weak.voice(heard.present, range, now);
                    // (Silences tell a skip from reading on, whatever the pointer's settings.)
                    self.marker.skipped().voice(heard.quiet, now);
                    if wants(settings).voice {
                        self.marker.voice(heard, loud, now);
                    }
                    self.voice_range = range;
                }
                Event::Audio(a) => self.meter.hear(a, now),
                Event::Failed(e) => {
                    // Listening stopped: the words skipped go with it.
                    self.marker.skipped().clear();
                    self.status = Status::Failed(e);
                }
            }
        }
        // Plans for settings that use them, and the voice level; asked again of a listener joined anew.
        let want = asked(settings);
        if self.status == Status::Listening && self.want != Some(want) {
            self.link.send_want(want);
            self.want = Some(want);
            (self.offers.since, self.offers.locked) = (now, false);
        }
        // The meter asks for frames while it moves (nothing at rest); the connecting spinner turns.
        if let Some(after) = self.meter.tick(now, ctx.pixels_per_point()) {
            ctx.request_repaint_after_secs(after);
        }
        if self.status == Status::Starting {
            ctx.request_repaint_after_secs(meter::FRAME);
        }
    }

    /// Moves and draws the pointer over `words` (the words on screen, in reading order) in `area`, shows its toast, and
    /// says how the reader should follow. `touched`: the reader moved the view themselves this frame (it is theirs
    /// until the recitation is followed on it). Call after the reader has painted.
    #[allow(clippy::too_many_arguments)]
    pub fn frame(&mut self, ctx: &egui::Context, settings: &Settings, colors: &Colors, chapters: &[Chapter], words: &[Slot], area: Rect, touched: bool) -> Steer {
        let now = ctx.input(|i| i.time);
        // The words on screen are where the listener looks first (a Bismillah line's places are the reader's own).
        let ayah_words = |w: &&Slot| w.0.1 > 0;
        if let (Some(a), Some(b)) = (words.iter().find(ayah_words), words.iter().rfind(ayah_words))
            && let Some(by_reader) = self.visible.frame((a.0, b.0), now)
        {
            self.link.send_visible(a.0, b.0, by_reader);
        }
        if touched {
            self.marker.reader_moved(words);
            self.line = None;
        }
        let layer = |name| egui::LayerId::new(egui::Order::Middle, egui::Id::unique(name));
        let stage = Stage { now, dt: frame_dt(ctx), area, layers: (layer("voice-range"), layer("voice-pointer")), scale: 1.0, scrolls: settings.auto_scroll };
        let shown = self.marker.frame(ctx, &stage, settings, colors, words);
        ask_frame(ctx, shown.wake);
        if let Some(slip) = shown.slip {
            // Back from a far place: a slip, or the listener's mistake (where a mistake notifier would
            // say so).
            crate::stats::note(|| format!("voice: heard at {} for {:.1} s while followed at {}, then back", key(slip.to), now - slip.since, key(slip.from)));
        }
        if let Some(at) = shown.at {
            let em = words.iter().find(|w| w.1.y_range().contains(at.y)).map_or(30.0, |w| w.1.height());
            self.line = Some((at.y, em));
        }
        let mut steer = shown.steer;
        // (A replay's is the recording's microphone, which nobody can move closer now.)
        let weak = self.status == Status::Listening && !self.replay && self.weak.tick(now);
        let skipped = self.marker.skipped().summary().filter(|_| settings.skipped.enabled);
        let note = Note::first(shown.offer, skipped, weak);
        let toast = note.as_ref().map(|note| Toast::new(ctx, note, chapters, colors, area.width() - 24.0, 1.0));
        let height = toast.as_ref().map_or(0.0, Toast::height);
        if let (Some(y), Some(toast)) = (self.toast.top(note.as_ref(), area, self.line, height), toast) {
            let skips = matches!(note, Some(Note::Skipped(_)));
            match toast.show(ctx, egui::Id::unique("voice-toast"), area, y, colors) {
                Some(Click::Go) => steer.go_to = steer.go_to.or(self.marker.choose(true)),
                Some(Click::Stay) => steer.go_to = steer.go_to.or(self.marker.choose(false)),
                // To a little before the first words skipped, as the pointer's moves go (a fade, a page turned).
                Some(Click::Back) => steer.go_to = steer.go_to.or(self.marker.skipped().back_to()),
                Some(Click::Dismiss) if skips => {
                    self.marker.skipped().clear();
                    ctx.request_repaint();
                }
                Some(Click::Dismiss) => self.weak.dismiss(),
                // The countdown moves on (the skipped words' toast changes only with news).
                None if !skips => ctx.request_repaint_after_secs(0.25),
                None => {}
            }
        }
        if let Some(to) = steer.go_to {
            self.visible.going((to.chapter, to.verse, to.word), now);
        }
        if let Some(surah) = steer.lead {
            self.visible.going((surah, 1, 1), now);
        }
        if steer.scroll != 0.0 {
            self.visible.scrolled();
        }
        steer.closed = std::mem::take(&mut self.closed);
        steer
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_listeners_news_marks_the_words_skipped_and_they_go_when_listening_stops() {
        let (link, news) = Link::fake();
        let mut voice = Voice::with(link, "", false);
        let (ctx, settings) = (egui::Context::default(), Settings::default());
        let poll = |voice: &mut Voice, now: f64| {
            let mut output = ctx.run_pass(egui::RawInput { time: Some(now), ..Default::default() }, |ctx| voice.poll(ctx, &settings));
            output.textures_delta.clear();
        };
        let say = |loc: Loc, word: u32, letter: u16, state: &str| {
            Event::Heard(link::Heard { loc, word, letter, letters: 4, state: state.into(), conf: 0.97, speed: 8.2, t: 0.0, slid: None, alts: Vec::new(), plan: None })
        };
        news.send(Event::Connected).unwrap();
        // 68:44's last words, a stop the listener reports, and on two seconds later with 68:46 (held, then followed):
        // 68:45 left out.
        let reports = [
            (0.32, say((68, 44, 8), 73_030, 0, "Locked")),
            (0.64, say((68, 44, 8), 73_030, 2, "Locked")),
            (0.96, say((68, 44, 9), 73_031, 0, "Locked")),
            (1.28, say((68, 44, 9), 73_031, 2, "Locked")),
            (1.6, say((68, 44, 10), 73_032, 0, "Locked")),
            (1.92, say((68, 44, 10), 73_032, 2, "Locked")),
            (2.24, say((68, 44, 10), 73_032, 3, "Paused")),
            (4.56, say((68, 46, 1), 73_038, 0, "Locked")),
            (4.88, say((68, 46, 1), 73_038, 2, "Locked")),
            (5.2, say((68, 46, 2), 73_039, 0, "Locked")),
        ];
        for (now, report) in reports {
            news.send(report).unwrap();
            poll(&mut voice, now);
        }
        let skipped = voice.marker.skipped().summary();
        assert_eq!(skipped, Some(skipped::Summary { from: (68, 45, 1), to: (68, 45, 5), words: 5, others: 0 }));
        // The listener stopped: the marks go with it.
        news.send(Event::Failed("gone".into())).unwrap();
        poll(&mut voice, 5.5);
        assert!(matches!(voice.status(), Status::Failed(_)) && voice.marker.skipped().is_empty());
    }
}
