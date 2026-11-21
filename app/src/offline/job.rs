//! Saving for offline use, on a thread of its own: a download of what the reader needs for the settings (the lists,
//! verse 1:1 and every surah's verses; a mushaf page or a juz is put together from those), or an update that asks for
//! every kept answer again; then, either way, the fonts the font cache lacks (see `steps`). It can be stopped, and a
//! download goes on where the last one stopped (what is kept is skipped).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use quran_core::api::Client;
use quran_core::fonts::FontSet;
use quran_core::parse::CHAPTER_COUNT;
use quran_core::settings::Settings;

use super::answer::api;
use super::copy::{self, needs};
use super::steps;
use super::store;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// What is not kept yet, for the settings.
    Download,
    /// Every kept answer, asked for again.
    Update,
}

#[derive(Debug, Clone, PartialEq)]
pub enum End {
    Done,
    Stopped,
    /// Why it stopped.
    Failed(String),
}

/// What a job is counting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    /// A download's surahs.
    Surahs,
    /// An update's kept answers.
    Answers,
    /// The page fonts, after either.
    Fonts,
}

/// How far a job has come.
#[derive(Debug, Clone)]
pub struct Progress {
    pub kind: Kind,
    pub part: Part,
    /// Done of `total` (0 while it is counted).
    pub done: usize,
    pub total: usize,
    /// How it ended, and when.
    pub end: Option<(End, Instant)>,
}

impl Progress {
    /// "37 of 114 surahs", "fonts 212 of 604", "120 of 412"; `None` while an update counts what is kept.
    pub fn count(&self) -> Option<String> {
        let (done, total) = (self.done, self.total);
        match self.part {
            Part::Surahs => Some(format!("{done} of {total} surahs")),
            Part::Fonts => Some(format!("fonts {done} of {total}")),
            Part::Answers => (total > 0).then(|| format!("{done} of {total}")),
        }
    }
}

pub struct Job {
    stopping: AtomicBool,
    progress: Mutex<Progress>,
}

/// The job under way, or the last one (one at a time).
static JOB: Mutex<Option<Arc<Job>>> = Mutex::new(None);

impl Job {
    /// A job of `kind`: a download counts surahs; an update, the kept answers it has yet to count.
    pub fn new(kind: Kind) -> Self {
        let (part, total) = match kind {
            Kind::Download => (Part::Surahs, usize::from(CHAPTER_COUNT)),
            Kind::Update => (Part::Answers, 0),
        };
        Self { stopping: AtomicBool::new(false), progress: Mutex::new(Progress { kind, part, done: 0, total, end: None }) }
    }

    pub fn progress(&self) -> MutexGuard<'_, Progress> {
        self.progress.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Goes on to counting `part`, of `total`.
    pub fn begin(&self, part: Part, total: usize) {
        let mut progress = self.progress();
        (progress.part, progress.done, progress.total) = (part, 0, total);
    }

    /// Ends the job; the first end counts (a stop shows at once, though the request under way goes on).
    fn end(&self, end: End) {
        let mut progress = self.progress();
        if progress.end.is_none() {
            progress.end = Some((end, Instant::now()));
        }
    }

    pub fn stopped(&self) -> bool {
        self.stopping.load(Ordering::Relaxed)
    }

    /// Stops it: at once on screen, and before its next request in the job.
    pub fn stop(&self) {
        self.stopping.store(true, Ordering::Relaxed);
        self.end(End::Stopped);
    }

    /// Waits `time`; `false` if the job was stopped meanwhile.
    pub fn wait(&self, time: Duration) -> bool {
        let until = Instant::now() + time;
        while !self.stopped() {
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return true;
            }
            std::thread::sleep(left.min(Duration::from_millis(50)));
        }
        false
    }
}

/// How the job under way, or the last one, has come along.
pub fn progress() -> Option<Progress> {
    JOB.lock().unwrap_or_else(PoisonError::into_inner).as_ref().map(|job| job.progress().clone())
}

/// Starts a job for `settings`, unless one is under way.
pub fn start(ctx: &egui::Context, kind: Kind, client: &Client, settings: &Settings) {
    let (Some(store), Some(dir)) = (store::user(), copy::font_dir()) else { return };
    let mut slot = JOB.lock().unwrap_or_else(PoisonError::into_inner);
    if slot.as_ref().is_some_and(|job| job.progress().end.is_none()) {
        return;
    }
    let job = Arc::new(Job::new(kind));
    *slot = Some(job.clone());
    drop(slot);
    let (ctx, needs, ran) = (ctx.clone(), needs(client, &settings.verse_query()), job.clone());
    let fonts = copy::fonts(settings.font_bases(), FontSet::of(settings.mushaf_font));
    let run = move || {
        let work = || match kind {
            Kind::Download => steps::download(&ran, store, &api, &needs, &ctx),
            Kind::Update => steps::update(&ran, store, &api, &ctx),
        };
        let then = || match work() {
            End::Done => steps::fonts(&ran, &dir, &api, &fonts, &ctx),
            end => end,
        };
        ran.end(crate::task::guarded(then, || End::Failed("it stopped unexpectedly".into())));
        ctx.request_repaint();
    };
    if let Err(e) = std::thread::Builder::new().name("offline".into()).spawn(run) {
        job.end(End::Failed(format!("it could not start ({e})")));
    }
}

/// Stops the job under way.
pub fn stop() {
    if let Some(job) = JOB.lock().unwrap_or_else(PoisonError::into_inner).as_ref() {
        job.stop();
    }
}
