//! Reading offline (desktop only; a browser has its own cache). The app keeps a copy of the API's answers on disk and
//! reads from it first, so a view opens at once, and without a connection; a copy of every surah can be saved in one
//! go, and updated.
//!
//! - `store`: the copy on disk.
//! - `answer`: a request's answer from the copy, from the surahs' answers put together, or from the API.
//! - `copy`: what a copy for the settings is made of, and how much of it is kept.
//! - `job`, `steps`: saving (a download for the settings, or an update of everything kept, then the fonts missing),
//!   one request at a time.
//! - `toast`: the first run's offer, and the saving's progress; `section`: the settings panel's part.

mod answer;
mod copy;
mod job;
mod section;
mod steps;
mod store;
mod toast;

use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use egui::Rect;
use quran_core::api::Client;
use quran_core::settings::Settings;

pub use self::answer::{answer, verses};
use self::copy::Survey;
use self::job::{End, Kind, Progress};
pub use self::section::Section;
use self::toast::{Click, Note};
use crate::task::{self, Poll, Pool};
use crate::theme::Colors;

/// How long "Saved for offline use" shows.
const SAVED_FOR: Duration = Duration::from_secs(4);

/// The toast's part: the first run's offer, and the saving's progress.
#[derive(Default)]
pub struct Offline {
    /// Whether everything is saved for the settings (`None`: not known yet), looked at once while the offer is to
    /// be made, and the look under way.
    whole: Option<bool>,
    survey: Option<Receiver<Option<Survey>>>,
    /// The end of the job that failed whose toast was dismissed.
    dismissed: Option<Instant>,
}

impl Offline {
    /// The toast, near the foot of `area` (none while the settings panel, which shows the same, is open). Returns
    /// whether the settings changed: the offer answered, or saving started in the panel instead.
    pub fn toast(&mut self, ctx: &egui::Context, area: Rect, settings: &mut Settings, colors: &Colors, client: &Client, hidden: bool) -> bool {
        let progress = job::progress();
        let note = match &progress {
            Some(progress) => self.note(ctx, progress),
            None => self.offer(ctx, settings, client).then_some(Note::Offer),
        };
        let mut not_now = false;
        if let Some(note) = note.filter(|_| !hidden) {
            match toast::show(ctx, &note, area, colors) {
                Some(Click::Download) => job::start(ctx, Kind::Download, client, settings),
                Some(Click::NotNow) => not_now = true,
                Some(Click::Retry) => {
                    if let Note::Failed(kind, _) = note {
                        job::start(ctx, kind, client, settings);
                    }
                }
                Some(Click::Stop) => job::stop(),
                Some(Click::Dismiss) => self.dismissed = progress.and_then(|p| p.end).map(|(_, at)| at),
                None => {}
            }
        }
        // Answered, or saving started (here or in the panel): the offer is not made again.
        let answered = !settings.offline_offered && (not_now || job::progress().is_some());
        settings.offline_offered |= answered;
        answered
    }

    /// What the toast says of a job: how far it has come, that it is done (for a moment), or that it failed (until
    /// dismissed).
    fn note(&self, ctx: &egui::Context, progress: &Progress) -> Option<Note> {
        match &progress.end {
            None => Some(Note::Saving(progress.kind, progress.count())),
            Some((End::Done, at)) => {
                ctx.request_repaint_after(SAVED_FOR.checked_sub(at.elapsed())?);
                Some(Note::Saved(progress.kind))
            }
            Some((End::Failed(why), at)) if self.dismissed != Some(*at) => Some(Note::Failed(progress.kind, why.clone())),
            Some(_) => None,
        }
    }

    /// Whether the first run's offer is made: it has not been answered, and not everything is saved.
    fn offer(&mut self, ctx: &egui::Context, settings: &Settings, client: &Client) -> bool {
        if settings.offline_offered {
            return false;
        }
        match self.survey.as_ref().map(task::poll) {
            Some(Poll::Done(survey)) => (self.whole, self.survey) = (Some(survey.is_none_or(|s| s.whole)), None),
            Some(Poll::Lost) => (self.whole, self.survey) = (Some(true), None),
            Some(Poll::Pending) => {}
            None if self.whole.is_none() => {
                let (client, settings) = (client.clone(), settings.clone());
                self.survey = Some(task::run(Pool::Net, ctx, move || copy::survey(&client, &settings)));
            }
            None => {}
        }
        self.whole == Some(false)
    }
}
