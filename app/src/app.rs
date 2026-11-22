//! The app: its state, the header, the current view and the settings panel.

use std::sync::mpsc::Receiver;

use egui::Ui;
use quran_core::api::{Chapter, Client, Juz, Verse, VerseQuery};
use quran_core::lists::clean_juzs;
use quran_core::settings::{self, LANGUAGE, Settings, storage};
use quran_core::view::{Location, Place, Position, Reading, View};

use crate::fonts::FontStore;
use crate::header::{self, HeaderAction, HeaderInput};
use crate::search::{Results, SearchBox};
use crate::lists::{self, Lists};
use crate::reader::{Env, Reader};
use crate::remote::{RETRY_AFTER, Remote};
use crate::pointer::{Lens, ReadingPointer};
use crate::settings_panel::Panel;
use crate::task::{self, Poll};
use crate::theme::Colors;

/// The location is saved this long after it last changed (and on exit).
const SAVE_DELAY: f64 = 0.5;

pub struct App {
    settings: Settings,
    /// As last written to disk.
    saved_settings: Settings,
    colors: Colors,
    client: Client,
    fonts: FontStore,
    chapters: Remote<Chapter>,
    juzs: Remote<Juz>,
    /// Verse 1:1 for the query it was fetched with (`None` if that failed, and
    /// when), and the request for it.
    bismillah: Option<(VerseQuery, Option<Verse>, f64)>,
    bismillah_job: Option<Receiver<(VerseQuery, Result<Verse, String>)>>,
    view: View,
    reader: Option<Reader>,
    /// The last place read, for the toggle's way back from a list.
    last_read: Option<Position>,
    lists: Lists,
    search: SearchBox,
    /// The search results on screen, in place of the view.
    results: Option<Results>,
    settings_open: bool,
    panel: Panel,
    pointer: ReadingPointer,
    /// Frame times, for the statistics.
    pub frames: crate::stats::Frames,
    /// The pointer's lens for this frame.
    lens: Option<Lens>,
    /// (Web) The font for letters the built-in one lacks, on its way.
    #[cfg(target_arch = "wasm32")]
    extra_font: Option<Receiver<Vec<u8>>>,
    /// (Web) The tab's title, as last set.
    #[cfg(target_arch = "wasm32")]
    title: String,
    /// The location as last written to disk (in a browser: to the address bar),
    /// the one to write, and since when it differs.
    saved_location: String,
    location: String,
    location_changed: f64,
    /// Recording the recitation (header button).
    #[cfg(not(target_arch = "wasm32"))]
    recorder: Option<crate::recorder::Recorder>,
    /// Following the recitation with the voice pointer (header button), or a replay of the last recording.
    #[cfg(not(target_arch = "wasm32"))]
    voice: Option<crate::voice::Voice>,
    /// Replaying the last recording (header button): `voice` follows it meanwhile.
    #[cfg(not(target_arch = "wasm32"))]
    replay: Option<crate::voice::Replay>,
    /// The voice pointer turning to another place: a fade, and the view it goes to loading out of sight.
    #[cfg(not(target_arch = "wasm32"))]
    turn: crate::turn::Turn,
    /// Saving for reading offline: the first run's offer and the progress (desktop only).
    #[cfg(not(target_arch = "wasm32"))]
    offline: crate::offline::Offline,
    /// A note for the title bar (a recording saved, the listener stopped) until a time.
    note: Option<(String, f64)>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let ctx = &cc.egui_ctx;
        let settings = storage::load().unwrap_or_default();
        // Where to start: the place the app was last at, or in a browser the address it was opened with.
        #[cfg(not(target_arch = "wasm32"))]
        let start = storage::load_location().and_then(|l| Place::parse(&l));
        #[cfg(target_arch = "wasm32")]
        let start = Place::parse(&crate::web::current());
        let start = start.unwrap_or(Place::At(View::SurahList.at(None)));
        let colors = Colors::of(&settings, ctx.system_theme() == Some(egui::Theme::Dark));
        colors.apply(ctx);
        let mut app = Self {
            saved_settings: settings.clone(),
            colors,
            client: Client::default(),
            fonts: FontStore::new(ctx, settings.font_bases(), FontStore::user_cache()),
            chapters: Remote::new(),
            juzs: Remote::new(),
            bismillah: None,
            bismillah_job: None,
            view: View::SurahList,
            reader: None,
            last_read: None,
            lists: Lists::default(),
            search: SearchBox::default(),
            results: None,
            settings_open: false,
            panel: Panel::new(),
            pointer: ReadingPointer::default(),
            frames: Default::default(),
            lens: None,
            #[cfg(target_arch = "wasm32")]
            extra_font: None,
            #[cfg(target_arch = "wasm32")]
            title: String::new(),
            saved_location: start.to_string(),
            location: start.to_string(),
            location_changed: 0.0,
            #[cfg(not(target_arch = "wasm32"))]
            recorder: None,
            #[cfg(not(target_arch = "wasm32"))]
            voice: None,
            #[cfg(not(target_arch = "wasm32"))]
            replay: None,
            #[cfg(not(target_arch = "wasm32"))]
            turn: Default::default(),
            #[cfg(not(target_arch = "wasm32"))]
            offline: Default::default(),
            note: None,
            settings,
        };
        app.load_lists(ctx);
        match start {
            Place::At(location) => app.go(ctx, location),
            Place::Search(query) => app.search_for(ctx, &query),
        }
        // `QURAN_EG_LISTEN=1`: follow the recitation from the start.
        #[cfg(not(target_arch = "wasm32"))]
        if std::env::var_os("QURAN_EG_LISTEN").is_some_and(|v| v == "1") {
            app.toggle_listening(ctx, 0.0);
        }
        #[cfg(target_arch = "wasm32")]
        {
            crate::web::listen(ctx);
            // Letters the built-in font lacks (ḥ, ṣ, ʿ): a font of the site, fetched once.
            let (tx, rx) = std::sync::mpsc::channel();
            crate::net::get(ctx, "fonts/NotoSans-Regular.ttf".into(), "*/*", move |fetched| {
                if let Ok((200..=299, bytes)) = fetched {
                    let _ = tx.send(bytes);
                }
            });
            app.extra_font = Some(rx);
        }
        app
    }

    fn load_lists(&mut self, ctx: &egui::Context) {
        self.chapters.load(ctx, self.client.chapters_call(LANGUAGE), |list| list);
        self.juzs.load(ctx, self.client.juzs_call(), clean_juzs);
    }

    fn env(&mut self) -> Env<'_> {
        Env {
            settings: &self.settings,
            colors: &self.colors,
            fonts: &mut self.fonts,
            chapters: &self.chapters.items,
            juzs: &self.juzs.items,
            client: &self.client,
            bismillah: self.bismillah.as_ref().and_then(|(_, verse, _)| verse.as_ref()),
            lens: self.lens,
        }
    }

    /// Go to a view because the reader asked to: in a browser, a new entry in the history.
    fn open(&mut self, ctx: &egui::Context, to: Location) {
        self.go(ctx, to);
        #[cfg(target_arch = "wasm32")]
        self.record(Place::At(to).to_string());
    }

    /// (Web) Put a place the app went to in the address bar, as a new history entry.
    #[cfg(target_arch = "wasm32")]
    fn record(&mut self, location: String) {
        if location != self.saved_location {
            crate::web::push(&location);
            self.location = location.clone();
            self.saved_location = location;
        }
    }

    /// Go to a view, at a reading position if the location has one.
    fn go(&mut self, ctx: &egui::Context, to: Location) {
        self.go_into(ctx, to, None);
    }

    /// Go to a view, in `loaded` if it is that view loaded already (out of sight, for the voice pointer).
    fn go_into(&mut self, ctx: &egui::Context, to: Location, loaded: Option<Reader>) {
        self.search.close();
        // Already there (the toggle's own button on a page view, a link to this view):
        // nothing is loaded again; a position in it is scrolled to.
        if to.view == self.view
            && self.results.is_none()
            && let Some(reader) = &mut self.reader
        {
            reader.go_to(to.at);
            return;
        }
        if let Some(position) = self.reader.as_ref().and_then(|r| r.position) {
            self.last_read = Some(position);
        }
        self.results = None;
        self.view = to.view;
        self.reader = match loaded.filter(|r| r.view == to.view) {
            Some(mut reader) => {
                reader.go_to(to.at);
                Some(reader)
            }
            None => Reader::open(to.view, to.at, ctx, &self.env()),
        };
        if self.view != View::SurahList {
            self.lists.release();
        }
    }

    /// Fetch verse 1:1 (the Bismillah line's words) for the current verse query.
    fn ensure_bismillah(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        let query = self.settings.verse_query();
        let done = match self.bismillah_job.as_ref().map(task::poll) {
            Some(Poll::Done(done)) => Some(done),
            Some(Poll::Lost) => Some((query.clone(), Err("the request ended unexpectedly".into()))),
            Some(Poll::Pending) | None => None,
        };
        if let Some((asked, result)) = done {
            self.bismillah_job = None;
            if let Err(e) = &result {
                eprintln!("quran-eg: could not load verse 1:1 for the Bismillah line: {e}");
            }
            self.bismillah = Some((asked, result.ok(), now));
        }
        let current = self.bismillah.as_ref().is_some_and(|(q, verse, at)| {
            // Have it, or failed too recently to ask again (the line shows its text meanwhile).
            *q == query && (verse.is_some() || now - at < RETRY_AFTER)
        });
        if self.reader.is_none() || self.bismillah_job.is_some() || current {
            return;
        }
        let call = self.client.verse_by_key_call("1:1", &query);
        self.bismillah_job = Some(crate::net::call(ctx, call, move |answer| (query, answer.map_err(|e| e.to_string()))));
    }

    /// Settings were changed in the panel: save them, and reload what depends on them.
    fn settings_changed(&mut self, ctx: &egui::Context) {
        storage::normalize(&mut self.settings);
        if self.settings == self.saved_settings {
            return;
        }
        let requery = self.settings.verse_query() != self.saved_settings.verse_query();
        storage::save(&self.settings);
        self.saved_settings = self.settings.clone();
        self.fonts.set_bases(self.settings.font_bases());
        if requery && let Some(mut reader) = self.reader.take() {
            reader.reload(ctx, &self.env());
            self.reader = Some(reader);
        }
    }

    /// Show the results for `query` because the reader asked to.
    fn show_results(&mut self, ctx: &egui::Context, query: &str) {
        self.search_for(ctx, query);
        #[cfg(target_arch = "wasm32")]
        self.record(Place::Search(query.trim().to_string()).to_string());
    }

    /// Show the results for `query` in place of the view (which is closed: its
    /// place is remembered for the way back).
    fn search_for(&mut self, ctx: &egui::Context, query: &str) {
        if let Some(position) = self.reader.as_ref().and_then(|r| r.position) {
            self.last_read = Some(position);
        }
        self.reader = None;
        self.lists.release();
        self.search.set_query(query);
        self.results = Some(Results::new(ctx, &self.client, query));
    }

    fn header(&mut self, ui: &mut Ui) -> Option<HeaderAction> {
        #[cfg(not(target_arch = "wasm32"))]
        let now = ui.input(|i| i.time);
        let here = self.reader.as_ref().and_then(|r| r.position);
        // Search results belong to no mode and have no title.
        let view = self.results.is_none().then_some(self.view);
        let reading = Reading { view, here, last: here.or(self.last_read), juzs: &self.juzs.items, chapters: &self.chapters.items };
        let input = HeaderInput {
            content_width: settings::content_width_px(self.settings.content_width) as f32,
            title: self.status().or_else(|| view.and_then(|v| v.title(&self.chapters.items))),
            mode: view.map(|v| v.mode()),
            targets: quran_core::view::Mode::ALL.map(|mode| reading.target(mode)),
            settings_open: self.settings_open,
            #[cfg(not(target_arch = "wasm32"))]
            recording: self.recorder.as_ref().map(|r| (r.elapsed(), r.start_word().to_string())),
            #[cfg(target_arch = "wasm32")]
            recording: None,
            // (A replay's voice is not the microphone's.)
            #[cfg(not(target_arch = "wasm32"))]
            listening: self.voice.as_ref().filter(|_| self.replay.is_none()).map(|v| v.listening(now)),
            #[cfg(target_arch = "wasm32")]
            listening: None,
            hifz: self.settings.hifz,
            #[cfg(not(target_arch = "wasm32"))]
            replay: self.replay.as_ref().map(|r| r.button(now)),
            #[cfg(target_arch = "wasm32")]
            replay: None,
        };
        let mut search = std::mem::take(&mut self.search);
        let action = header::show(ui, &mut self.env(), &mut search, input);
        self.search = search;
        action
    }

    /// Remember where the reader is; written to disk once it has stayed put a moment.
    fn track_location(&mut self, ctx: &egui::Context) {
        let location = match (&self.reader, &self.results) {
            (Some(reader), _) => reader.location().to_string(),
            // In a browser the results have an address; natively they are not a place to
            // come back to, and the view under them stays the location.
            #[cfg(target_arch = "wasm32")]
            (None, Some(results)) => Place::Search(results.query.clone()).to_string(),
            #[cfg(not(target_arch = "wasm32"))]
            (None, Some(_)) => self.location.clone(),
            (None, None) => self.view.at(None).to_string(),
        };
        let now = ctx.input(|i| i.time);
        if location != self.location {
            self.location = location;
            self.location_changed = now;
        }
        if self.location != self.saved_location {
            if now - self.location_changed >= SAVE_DELAY {
                self.save_location();
            } else {
                ctx.request_repaint_after_secs(SAVE_DELAY as f32);
            }
        }
    }

    /// What the title bar says instead of the view's title: a replay, recording, listening, or a note.
    fn status(&self) -> Option<String> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            if let Some(replay) = &self.replay {
                return Some(replay.title(self.voice.as_ref()));
            }
            let listening = self.voice.as_ref().map(|v| v.summary());
            if let Some(r) = &self.recorder {
                let rec = format!("● Recording from {} · {:.0}:{:02.0}", r.start_word(), (r.elapsed() / 60.0).floor(), r.elapsed() % 60.0);
                return Some(listening.map_or(rec.clone(), |l| format!("{rec} · {}", l.trim_start_matches("◉ Listening · "))));
            }
            if listening.is_some() {
                return listening;
            }
        }
        self.note.as_ref().map(|(note, _)| note.clone())
    }

    /// Starts following the recitation (joining or starting quran-voice's listener), or stops. During a replay: the
    /// replay ends, and listening starts.
    #[cfg(not(target_arch = "wasm32"))]
    fn toggle_listening(&mut self, ctx: &egui::Context, now: f64) {
        // The header, painted before this, shows the change (the spinner, or off) on the next frame.
        ctx.request_repaint();
        if self.replay.take().is_some() {
            self.voice = None;
        } else if self.voice.take().is_some() {
            return;
        }
        self.listen(ctx, now);
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn listen(&mut self, ctx: &egui::Context, now: f64) {
        match crate::voice::Voice::start(&self.settings.voice, ctx) {
            Ok(voice) => self.voice = Some(voice),
            Err(e) => self.note = Some((format!("Listening: {e}"), now + 10.0)),
        }
    }

    /// A listener this app started runs the model the settings name: chosen anew, it starts again with it (one joined
    /// keeps its own). A replay keeps its model; listening after it starts with the new one.
    #[cfg(not(target_arch = "wasm32"))]
    fn follow_model(&mut self, ctx: &egui::Context, now: f64) {
        let other = self.voice.as_ref().is_some_and(|v| !v.joined() && v.model() != self.settings.voice.model);
        if other && self.replay.is_none() {
            crate::stats::note(|| format!("voice: the listener starts again with {}", settings::model_folder(&self.settings.voice.model)));
            self.voice = None;
            self.listen(ctx, now);
        }
    }

    /// Replays the newest recording with the voice pointer following it, from where it was recorded, or stops the
    /// replay (or calls it off while it starts). Live listening stops for it and comes back after. A recording under
    /// way is saved first, and is the one replayed.
    #[cfg(not(target_arch = "wasm32"))]
    fn toggle_replay(&mut self, ctx: &egui::Context, now: f64) {
        ctx.request_repaint();
        if self.replay.is_some() {
            return self.end_replay(ctx, now, None);
        }
        if self.recorder.is_some() {
            self.toggle_recording(ctx, now);
        }
        let Some(wav) = crate::recorder::newest() else {
            self.note = Some(("Replay: no recording yet (Record makes one)".into(), now + 6.0));
            return;
        };
        // (The live listener lets its model go before the replay's loads.)
        let resume = self.voice.take().is_some();
        match crate::voice::Replay::start(&self.settings.voice, ctx, &wav, resume) {
            Ok((replay, voice)) => {
                if let Some((at, page)) = replay.start {
                    let view = match self.view {
                        View::Page(_) if page > 0 => View::Page(page),
                        view => view.holding(at, &self.juzs.items),
                    };
                    self.go(ctx, view.at(Some(at)));
                }
                (self.replay, self.voice) = (Some(replay), Some(voice));
            }
            Err(e) => {
                self.note = Some((format!("Replay: {e}"), now + 10.0));
                if resume {
                    self.listen(ctx, now);
                }
            }
        }
    }

    /// The replay is over (stopped, called off, played out, or failed: `note` says why): its sound and its listener
    /// stop, and listening comes back if it was on.
    #[cfg(not(target_arch = "wasm32"))]
    fn end_replay(&mut self, ctx: &egui::Context, now: f64, note: Option<String>) {
        let Some(replay) = self.replay.take() else { return };
        self.voice = None;
        let resume = replay.resume;
        drop(replay);
        if resume {
            self.listen(ctx, now);
        }
        if let Some(note) = note {
            self.note = Some((note, now + 10.0));
        }
        ctx.request_repaint();
    }

    /// The replay's frame: its clock in the title bar moves on, and it ends once played out.
    #[cfg(not(target_arch = "wasm32"))]
    fn tick_replay(&mut self, ctx: &egui::Context, now: f64) {
        let Some(replay) = self.replay.as_mut() else { return };
        match replay.tick(self.voice.as_ref(), now) {
            crate::voice::Tick::Going => ctx.request_repaint_after_secs(0.25),
            crate::voice::Tick::Over(note) => self.end_replay(ctx, now, note),
        }
    }

    /// The listener's news, before the header shows it; a listener that failed is dropped (a replay's when the
    /// replay ends).
    #[cfg(not(target_arch = "wasm32"))]
    fn hear_voice(&mut self, ctx: &egui::Context, now: f64) {
        let Some(voice) = self.voice.as_mut() else { return };
        voice.poll(ctx, &self.settings.voice);
        if let crate::voice::Status::Failed(e) = voice.status()
            && self.replay.is_none()
        {
            self.note = Some((format!("Listening stopped: {e}"), now + 10.0));
            self.voice = None;
        }
    }

    /// The voice pointer's frame, after the reader has painted: it draws itself and steers the reader, scrolling along
    /// or asking for a move to the place being read: in this view, or in the view of the mode the reader reads in that
    /// holds it (a page for a page), with the surah whose head to show there (a basmala leading into it).
    #[cfg(not(target_arch = "wasm32"))]
    fn follow_voice(&mut self, ctx: &egui::Context, ground: egui::Rect, now: f64, touched: bool) -> Option<crate::turn::Move> {
        let opacity = self.turn.opacity(now);
        let voice = self.voice.as_mut()?;
        voice.set_opacity(opacity);
        let words = self.reader.as_ref().filter(|_| self.results.is_none()).map_or(&[][..], |r| &r.words[..]);
        let touched = touched || self.reader.as_ref().is_some_and(|r| r.touched);
        let steer = voice.frame(ctx, &self.settings.voice, &self.colors, &self.chapters.items, words, ground, touched);
        // (A replay goes on to the recording's end.)
        if steer.closed && self.settings.voice.stop_on_closing && self.replay.is_none() {
            self.note = Some(("Listening stopped: sadaqa-llahu-l-'azim".into(), now + 5.0));
            self.voice = None;
            // The header, already painted, still shows the microphone on.
            ctx.request_repaint();
            return None;
        }
        let reader = self.reader.as_mut()?;
        if steer.scroll != 0.0 {
            reader.scroll_by(steer.scroll);
        }
        let (at, head) = match (steer.lead, steer.go_to) {
            (Some(chapter), _) => (quran_core::view::WordRef { chapter, verse: 1, word: 1 }, Some(chapter)),
            (None, Some(at)) => (at, None),
            (None, None) => return None,
        };
        // In this view (a surah's or a juz's loads up to it), or the view of the same mode that holds it.
        let here = match self.view {
            View::Surah { chapter, .. } => chapter == at.chapter,
            View::Juz(juz) => quran_core::lists::juz_of(&self.juzs.items, at.chapter, at.verse) == Some(juz),
            _ => false,
        };
        let view = if here || reader.has_word(at) { self.view } else { self.view.holding(at, &self.juzs.items) };
        Some(crate::turn::Move { to: view.at(Some(at)), head })
    }

    /// The voice pointer's part in the frame: its move faded through (the view it goes to loading out of sight
    /// meanwhile), and the next page loading while the reader is on a page's last lines. `go`: where the reader went
    /// themselves, which calls off a move.
    #[cfg(not(target_arch = "wasm32"))]
    fn voice_moves(&mut self, ctx: &egui::Context, ground: egui::Rect, now: f64, go: bool) {
        if go {
            self.turn.cancel();
        }
        let asked = self.follow_voice(ctx, ground, now, go).filter(|_| !go);
        if self.voice.is_none() && !self.turn.going() {
            self.turn.cancel();
        }
        let mut turn = std::mem::take(&mut self.turn);
        match asked {
            // The view just come in, not yet showing the place: the same move, made at once.
            Some(to) if to.to.view == self.view && turn.settling(now) => self.arrive(to),
            Some(to) => turn.start(to, self.view, now, ctx, &self.env()),
            None => {}
        }
        if let (View::Page(n), Some(voice), Some(reader)) = (self.view, &self.voice, &self.reader)
            && n < quran_core::nav::PAGE_COUNT
            && voice.seen().is_some_and(|at| reader.on_last_lines(at, 2))
        {
            turn.preload(View::Page(n + 1), ctx, &self.env());
        }
        let due = turn.due(now);
        let fading = turn.fading(now);
        self.turn = turn;
        if let Some((to, loaded)) = due {
            self.go_into(ctx, to.to, loaded);
            self.arrive(to);
        }
        if fading {
            ctx.request_repaint();
        }
    }

    /// The voice pointer's move, in the view it goes to (shown): at its place, or a surah's head.
    #[cfg(not(target_arch = "wasm32"))]
    fn arrive(&mut self, to: crate::turn::Move) {
        let Some(reader) = &mut self.reader else { return };
        match to.head {
            Some(chapter) => reader.go_to_head(chapter),
            None => reader.go_to(to.to.at),
        }
    }

    /// Starts recording at the reading position, or stops and saves (for `qv enroll`). A replay ends first: its sound
    /// is not the reader's.
    #[cfg(not(target_arch = "wasm32"))]
    fn toggle_recording(&mut self, ctx: &egui::Context, now: f64) {
        self.end_replay(ctx, now, None);
        let result = match self.recorder.take() {
            Some(r) => r.finish().map(|wav| format!("Saved {}", wav.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned()))),
            None => match self.reader.as_ref().and_then(|r| r.position) {
                Some(at) => crate::recorder::Recorder::start(&at).map(|r| {
                    self.recorder = Some(r);
                    String::new()
                }),
                None => Err("Open a surah, juz or page to record from".into()),
            },
        };
        self.note = match result {
            Ok(note) if note.is_empty() => None,
            Ok(note) => Some((note, now + 5.0)),
            Err(e) => Some((format!("Recording: {e}"), now + 8.0)),
        };
    }

    /// Notes reading-position changes while recording; stops on a microphone error; keeps the timer ticking.
    fn track_recording(&mut self, ctx: &egui::Context, now: f64) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(r) = &mut self.recorder {
            r.track(self.reader.as_ref().and_then(|x| x.position.as_ref()));
            if let Some(e) = r.error() {
                let r = self.recorder.take().expect("recording");
                let saved = r.finish().map_or_else(|f| format!(" ({f})"), |_| " (saved what was recorded)".into());
                self.note = Some((format!("Microphone error: {e}{saved}"), now + 10.0));
            }
            ctx.request_repaint_after_secs(0.25);
        }
        if self.note.as_ref().is_some_and(|(_, until)| now > *until) {
            self.note = None;
        }
        if let Some((_, until)) = &self.note {
            ctx.request_repaint_after_secs((*until - now).max(0.1) as f32);
        }
    }

    fn save_location(&mut self) {
        if self.location != self.saved_location {
            // The same history entry, at another reading position.
            #[cfg(target_arch = "wasm32")]
            crate::web::replace(&self.location);
            storage::save_location(&self.location);
            self.saved_location = self.location.clone();
        }
    }
}

/// What only the web build does each frame.
#[cfg(target_arch = "wasm32")]
impl App {
    fn browser(&mut self, ctx: &egui::Context) {
        // Work that native builds give to threads: a few milliseconds of it per frame.
        task::pump(ctx);
        // Back or forward: go where the address bar now is, without a new history entry.
        if crate::web::popped() {
            let current = crate::web::current();
            match Place::parse(&current) {
                Some(Place::At(location)) => self.go(ctx, location),
                Some(Place::Search(query)) => self.search_for(ctx, &query),
                None => self.go(ctx, View::SurahList.at(None)),
            }
            self.location = current.clone();
            self.saved_location = current;
        }
        if let Some(Ok(bytes)) = self.extra_font.as_ref().map(|rx| rx.try_recv()) {
            self.extra_font = None;
            use egui::epaint::text::{FontInsert, FontPriority, InsertFontFamily};
            let families = [egui::FontFamily::Proportional, egui::FontFamily::Monospace]
                .map(|family| InsertFontFamily { family, priority: FontPriority::Lowest });
            ctx.add_font(FontInsert::new("noto-sans", egui::FontData::from_owned(bytes), families.to_vec()));
        }
        // The tab is named after the view.
        let title = match (&self.results, self.view.title(&self.chapters.items)) {
            (Some(_), _) => "Search – The Noble Quran".to_string(),
            (None, Some(title)) => format!("{title} – The Noble Quran"),
            (None, None) => "The Noble Quran".to_string(),
        };
        if title != self.title {
            crate::web::set_title(&title);
            self.title = title;
        }
    }
}

impl App {
    /// One frame of the app.
    fn frame(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        #[cfg(target_arch = "wasm32")]
        self.browser(&ctx);
        let now = ctx.input(|i| i.time);
        self.fonts.poll();
        self.chapters.poll(now);
        self.juzs.poll(now);
        // Names and juz ranges are needed by every view: keep trying in the background.
        if self.chapters.due(now) || self.juzs.due(now) {
            self.load_lists(&ctx);
        }
        self.ensure_bismillah(&ctx);
        let colors = Colors::of(&self.settings, ctx.system_theme() == Some(egui::Theme::Dark));
        if colors != self.colors {
            self.colors = colors;
            colors.apply(&ctx);
        }

        #[cfg(not(target_arch = "wasm32"))]
        self.hear_voice(&ctx, now);
        let mut go = None;
        let bar = egui::Panel::top("header").exact_size(header::HEIGHT).frame(egui::Frame::new()).show_separator_line(false);
        match bar.show(ui, |ui| self.header(ui)).inner {
            Some(HeaderAction::Open(to)) => go = Some(to),
            Some(HeaderAction::Results(query)) => self.show_results(&ctx, &query),
            Some(HeaderAction::ToggleSettings) => self.settings_open = !self.settings_open,
            Some(HeaderAction::ToggleHifz) => {
                self.settings.hifz = !self.settings.hifz;
                self.settings_changed(&ctx);
            }
            #[cfg(not(target_arch = "wasm32"))]
            Some(HeaderAction::ToggleRecord) => self.toggle_recording(&ctx, now),
            #[cfg(not(target_arch = "wasm32"))]
            Some(HeaderAction::ToggleListen) => self.toggle_listening(&ctx, now),
            #[cfg(not(target_arch = "wasm32"))]
            Some(HeaderAction::ToggleReplay) => self.toggle_replay(&ctx, now),
            #[cfg(target_arch = "wasm32")]
            Some(HeaderAction::ToggleRecord | HeaderAction::ToggleListen | HeaderAction::ToggleReplay) => {}
            None => {}
        }
        #[cfg(not(target_arch = "wasm32"))]
        self.tick_replay(&ctx, now);
        self.track_recording(&ctx, now);

        // Everything below the header is the pointer's ground.
        let ground = ui.available_rect_before_wrap();
        let line_height = self.reader.as_ref().map_or_else(
            || settings::mushaf_em(self.settings.arabic_font_scale) * settings::MUSHAF_LINE,
            |reader| reader.line_height,
        );
        self.lens = self.pointer.lens(&ctx, &self.settings.pointer, ground, line_height);
        // A glowing voice pointer lights the words under it instead.
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(lens) = self.voice.as_ref().and_then(|v| v.lens(&self.settings.voice)) {
            self.lens = Some(lens);
        }

        let mut retry = false;
        // Fading for the voice pointer's move, and the view it goes to loading out of sight.
        #[cfg(not(target_arch = "wasm32"))]
        let (opacity, mut unseen) = (self.turn.opacity(now), self.turn.loading());
        #[cfg(not(target_arch = "wasm32"))]
        self.fonts.pin(unseen.as_ref().map_or_else(|| self.turn.pages(), crate::reader::Reader::pages));
        egui::CentralPanel::default().frame(egui::Frame::new().fill(self.colors.bg)).show(ui, |ui| {
            #[cfg(not(target_arch = "wasm32"))]
            let whole = ui.max_rect();
            #[cfg(not(target_arch = "wasm32"))]
            ui.multiply_opacity(opacity);
            let (loading, errors) = (
                (self.chapters.loading(), self.juzs.loading()),
                (self.chapters.error.clone(), self.juzs.error.clone()),
            );
            let mut reader = self.reader.take();
            let mut results = self.results.take();
            let mut lists = std::mem::take(&mut self.lists);
            let view = self.view;
            let mut env = self.env();
            let opened = match (&mut results, view) {
                (Some(results), _) => {
                    let chapters = env.chapters;
                    results.ui(ui, &mut env, chapters)
                }
                (None, View::SurahList) => lists.surahs(ui, &mut env, loading.0, errors.0.as_deref(), &mut retry),
                (None, View::JuzList) => lists::juzs(ui, &env, loading.1, errors.1.as_deref(), &mut retry),
                (None, _) => reader.as_mut().and_then(|r| r.ui(ui, &mut env)),
            };
            #[cfg(not(target_arch = "wasm32"))]
            if let Some(next) = unseen.as_mut() {
                // Drawn unseen and untouchable: it lays itself out and asks for its glyphs.
                let mut hidden = ui.new_child(egui::UiBuilder::new().max_rect(whole).invisible());
                next.ui(&mut hidden, &mut env);
            }
            self.reader = reader;
            self.results = results;
            self.lists = lists;
            go = go.or(opened);
        });
        #[cfg(not(target_arch = "wasm32"))]
        self.turn.loaded(unseen);

        if self.settings_open {
            let screen = ctx.content_rect();
            let page_em = line_height / settings::MUSHAF_LINE;
            #[cfg(not(target_arch = "wasm32"))]
            {
                self.panel.listener = self.voice.as_ref().and_then(|v| v.lacks(&self.settings.voice, now));
                self.panel.joined = self.replay.is_none() && self.voice.as_ref().is_some_and(crate::voice::Voice::joined);
            }
            if self.panel.show(&ctx, screen, &mut self.settings, &self.colors, &self.client, &mut self.fonts, page_em) {
                self.settings_open = false;
            }
            self.settings_changed(&ctx);
            #[cfg(not(target_arch = "wasm32"))]
            self.follow_model(&ctx, now);
        } else {
            self.panel.closed();
        }
        // The offer to save for reading offline, and its progress, near the foot of the reading area.
        #[cfg(not(target_arch = "wasm32"))]
        if self.offline.toast(&ctx, ground, &mut self.settings, &self.colors, &self.client, self.settings_open) {
            self.settings_changed(&ctx);
        }
        if retry {
            self.load_lists(&ctx);
        }
        // The reader going elsewhere themselves (a list, the search, a page's number) moves the view as a page turn does;
        // the pointer's own move is theirs to override.
        #[cfg(not(target_arch = "wasm32"))]
        self.voice_moves(&ctx, ground, now, go.is_some());
        if let Some(to) = go {
            self.open(&ctx, to);
            ctx.request_repaint();
        }
        self.track_location(&ctx);
        // Last: where the pointer draws itself, it hides the cursor the widgets asked for.
        self.pointer.show(&ctx, &self.settings.pointer, &self.colors, ground, line_height);
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let mut frames = std::mem::take(&mut self.frames);
        frames.time(&ui.ctx().clone(), || self.frame(ui));
        self.frames = frames;
        crate::stats::milestone("first-frame");
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // Stops the listener if this app started it, and a replay's sound.
        #[cfg(not(target_arch = "wasm32"))]
        drop((self.voice.take(), self.replay.take()));
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(r) = self.recorder.take() {
            match r.finish() {
                Ok(wav) => eprintln!("recording saved: {}", wav.display()),
                Err(e) => eprintln!("recording not saved: {e}"),
            }
        }
        self.save_location();
        storage::save(&self.settings);
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        self.colors.bg.to_normalized_gamma_f32()
    }
}
