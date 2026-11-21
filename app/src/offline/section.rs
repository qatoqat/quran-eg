//! The settings panel's Offline section: what is saved for reading offline, Download (or the rest of it) and Update,
//! and the saving's progress with Cancel.

use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant, SystemTime};

use egui::{CornerRadius, Rect, RichText, Sense, Ui, vec2};
use quran_core::api::{Client, VerseQuery};
use quran_core::parse::{CHAPTER_COUNT, MUSHAF_PAGES};
use quran_core::settings::{MushafFont, Settings};

use super::copy::{self, Survey};
use super::job::{self, End, Kind, Progress};
use crate::kit::{self, Pill};
use crate::settings_panel::{help, legend};
use crate::task::{self, Poll, Pool};
use crate::theme::Colors;

/// While the section shows, what is saved is looked at again this often (the reader keeps answers as it goes).
const AGAIN: Duration = Duration::from_secs(5);

#[derive(Default)]
pub struct Section {
    /// What is saved (`None`: there is no cache folder), and what that was looked at for.
    seen: Option<(Option<Survey>, Asked)>,
    survey: Option<(Receiver<Option<Survey>>, Asked)>,
}

/// What a survey was made for: the verse query and the mushaf font, the end of the last job then, and when.
#[derive(Clone, PartialEq)]
struct Asked {
    query: VerseQuery,
    font: MushafFont,
    job: Option<Instant>,
    at: Instant,
}

impl Section {
    pub fn show(&mut self, ui: &mut Ui, colors: &Colors, settings: &Settings, client: &Client) {
        legend(ui, "Offline");
        let progress = job::progress();
        match progress.as_ref().filter(|p| p.end.is_none()) {
            Some(p) => running(ui, colors, p),
            None => {
                self.look(ui.ctx(), settings, client, progress.as_ref());
                self.saved(ui, colors, settings, client, progress.as_ref());
            }
        }
        help(
            ui,
            colors,
            "Keeps the text of every surah, with its word meanings and the translations chosen above, and the mushaf \
             font's 604 page fonts on this computer (about 90 MB with the tajweed fonts and one translation): the \
             reader opens it at once and reads it without a connection. The text is asked for again in the background \
             once it is a week old; Update asks for all of it now (and for any font missing).",
        );
    }

    /// Looks at what is saved: when the settings or the last job have changed since, or a while has passed.
    fn look(&mut self, ctx: &egui::Context, settings: &Settings, client: &Client, progress: Option<&Progress>) {
        if let Some((rx, asked)) = &self.survey {
            match task::poll(rx) {
                Poll::Done(survey) => self.seen = Some((survey, asked.clone())),
                Poll::Lost => self.seen = Some((self.seen.take().and_then(|(survey, _)| survey), asked.clone())),
                Poll::Pending => return,
            }
            self.survey = None;
        }
        let job = progress.and_then(|p| p.end.as_ref()).map(|(_, at)| *at);
        let asked = Asked { query: settings.verse_query(), font: settings.mushaf_font, job, at: Instant::now() };
        let same = |seen: &Asked| (&seen.query, seen.font, seen.job) == (&asked.query, asked.font, asked.job);
        if !self.seen.as_ref().is_some_and(|(_, seen)| same(seen) && seen.at.elapsed() < AGAIN) {
            let (client, settings) = (client.clone(), settings.clone());
            self.survey = Some((task::run(Pool::Net, ctx, move || copy::survey(&client, &settings)), asked));
        }
        ctx.request_repaint_after(AGAIN);
    }

    fn saved(&self, ui: &mut Ui, colors: &Colors, settings: &Settings, client: &Client, last: Option<&Progress>) {
        let Some((survey, _)) = &self.seen else {
            help(ui, colors, "…");
            return;
        };
        let Some(survey) = survey else {
            help(ui, colors, "There is no cache folder on this computer to keep it in.");
            return;
        };
        for line in summary(survey) {
            ui.label(line);
        }
        if let Some((End::Failed(why), _)) = last.and_then(|p| p.end.as_ref()) {
            ui.label(RichText::new(format!("Stopped: {why}.")).size(13.6).color(colors.danger));
        }
        ui.horizontal(|ui| {
            let download = if survey.surahs + survey.fonts > 0 { "Download the rest" } else { "Download" };
            if !survey.whole && kit::pill(ui, colors, download, Pill::Small).clicked() {
                job::start(ui.ctx(), Kind::Download, client, settings);
            }
            if survey.surahs > 0 && kit::pill(ui, colors, "Update", Pill::Small).clicked() {
                job::start(ui.ctx(), Kind::Update, client, settings);
            }
        });
    }
}

fn running(ui: &mut Ui, colors: &Colors, p: &Progress) {
    let doing = if p.kind == Kind::Download { "Saving" } else { "Updating" };
    ui.label(p.count().map_or_else(|| format!("{doing}…"), |count| format!("{doing} · {count}")));
    // The bar: the border's colour, filled with the accent as far as it has come.
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 6.0), Sense::hover());
    let done = Rect::from_min_size(rect.min, vec2(rect.width() * p.done as f32 / p.total.max(1) as f32, rect.height()));
    ui.painter().rect_filled(rect, CornerRadius::same(3), colors.border);
    ui.painter().rect_filled(done, CornerRadius::same(3), colors.accent);
    if kit::pill(ui, colors, "Cancel", Pill::Small).clicked() {
        job::stop();
    }
}

/// What is saved, a line for the surahs' text and one for the page fonts: "114 surahs saved · 42 MB · updated 2 days
/// ago", "604 page fonts saved · 48 MB".
fn summary(survey: &Survey) -> Vec<String> {
    if survey.surahs + survey.fonts == 0 {
        return vec!["Not saved yet".into()];
    }
    let of = |n: usize, all: usize, what: &str| if n == all { format!("{all} {what} saved") } else { format!("{n} of {all} {what} saved") };
    let mut surahs = format!("{} · {}", of(survey.surahs, usize::from(CHAPTER_COUNT), "surahs"), megabytes(survey.bytes));
    if let Some(at) = survey.oldest {
        surahs += &format!(" · updated {}", ago(at));
    }
    vec![surahs, format!("{} · {}", of(survey.fonts, usize::from(MUSHAF_PAGES), "page fonts"), megabytes(survey.font_bytes))]
}

fn megabytes(bytes: u64) -> String {
    let mb = bytes as f64 / 1e6;
    if mb < 10.0 { format!("{mb:.1} MB") } else { format!("{mb:.0} MB") }
}

/// How long ago `at` was, roughly.
fn ago(at: SystemTime) -> String {
    let secs = at.elapsed().unwrap_or_default().as_secs();
    let (n, unit) = match secs {
        0..60 => return "just now".into(),
        60..3600 => (secs / 60, "minute"),
        3600..86400 => (secs / 3600, "hour"),
        86400..172800 => return "yesterday".into(),
        _ => (secs / 86400, "day"),
    };
    format!("{n} {unit}{} ago", if n == 1 { "" } else { "s" })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_is_saved_in_a_line() {
        let hours = |n: u64| Some(SystemTime::now() - Duration::from_secs(n * 3600));
        assert_eq!(summary(&Survey::default()), ["Not saved yet"]);
        let all = Survey { surahs: 114, fonts: 604, whole: true, oldest: hours(49), bytes: 42_300_000, font_bytes: 48_000_000 };
        assert_eq!(summary(&all), ["114 surahs saved · 42 MB · updated 2 days ago", "604 page fonts saved · 48 MB"]);
        let some = Survey { surahs: 37, fonts: 212, oldest: hours(0), bytes: 9_460_000, font_bytes: 17_000_000, ..all };
        assert_eq!(summary(&some), ["37 of 114 surahs saved · 9.5 MB · updated just now", "212 of 604 page fonts saved · 17 MB"]);
        assert_eq!((ago(hours(1).unwrap()), ago(hours(25).unwrap())), ("1 hour ago".into(), "yesterday".into()));
    }
}
