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
            settings,
        };
        app.load_lists(ctx);
        match start {
            Place::At(location) => app.go(ctx, location),
            Place::Search(query) => app.search_for(ctx, &query),
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
        self.reader = Reader::open(to.view, to.at, ctx, &self.env());
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
        let here = self.reader.as_ref().and_then(|r| r.position);
        // Search results belong to no mode and have no title.
        let view = self.results.is_none().then_some(self.view);
        let reading = Reading { view, here, last: here.or(self.last_read), juzs: &self.juzs.items, chapters: &self.chapters.items };
        let input = HeaderInput {
            content_width: settings::content_width_px(self.settings.content_width) as f32,
            title: view.and_then(|v| v.title(&self.chapters.items)),
            mode: view.map(|v| v.mode()),
            targets: quran_core::view::Mode::ALL.map(|mode| reading.target(mode)),
            settings_open: self.settings_open,
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

        let mut go = None;
        let bar = egui::Panel::top("header").exact_size(header::HEIGHT).frame(egui::Frame::new()).show_separator_line(false);
        match bar.show(ui, |ui| self.header(ui)).inner {
            Some(HeaderAction::Open(to)) => go = Some(to),
            Some(HeaderAction::Results(query)) => self.show_results(&ctx, &query),
            Some(HeaderAction::ToggleSettings) => self.settings_open = !self.settings_open,
            None => {}
        }

        // Everything below the header is the pointer's ground.
        let ground = ui.available_rect_before_wrap();
        let line_height = self.reader.as_ref().map_or_else(
            || settings::mushaf_em(self.settings.arabic_font_scale) * settings::MUSHAF_LINE,
            |reader| reader.line_height,
        );
        self.lens = self.pointer.lens(&ctx, &self.settings.pointer, ground, line_height);

        let mut retry = false;
        egui::CentralPanel::default().frame(egui::Frame::new().fill(self.colors.bg)).show(ui, |ui| {
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
            self.reader = reader;
            self.results = results;
            self.lists = lists;
            go = go.or(opened);
        });

        if self.settings_open {
            let screen = ctx.content_rect();
            if self.panel.show(&ctx, screen, &mut self.settings, &self.colors, &self.client) {
                self.settings_open = false;
            }
            self.settings_changed(&ctx);
        }
        if retry {
            self.load_lists(&ctx);
        }
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
        frames.time(|| self.frame(ui));
        self.frames = frames;
        crate::stats::milestone("first-frame");
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.save_location();
        storage::save(&self.settings);
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        self.colors.bg.to_normalized_gamma_f32()
    }
}
