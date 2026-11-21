//! What a job does, request by request: a download of what is not kept, an update of what is, and then the fonts the
//! font cache lacks. One request at a time with a pause between; quran.com unreachable or busy is waited for a
//! little (twice), then the job stops and says why.

use std::path::Path;
use std::time::Duration;

use quran_core::fonts::{cache_name, join};
use quran_core::settings::storage::write_atomic;

use super::answer::Fetch;
use super::copy::{FontFile, Need};
use super::job::{End, Job, Part};
use super::store::{Store, holds};

/// Between two requests.
const PAUSE: Duration = Duration::from_millis(300);
/// Before asking again, while quran.com can't be reached or is busy.
const WAITS: [Duration; 2] = [Duration::from_secs(2), Duration::from_secs(8)];

/// Asks for `url`, three times at most while quran.com can't be reached or is busy (waiting longer each time).
fn ask(job: &Job, fetch: Fetch, url: &str) -> Result<(u16, Vec<u8>), End> {
    let mut why = String::new();
    for wait in std::iter::once(Duration::ZERO).chain(WAITS) {
        if !job.wait(wait) {
            return Err(End::Stopped);
        }
        match fetch(url) {
            Ok((status, _)) if status == 429 || status >= 500 => why = format!("quran.com is busy (HTTP {status})"),
            Ok(answer) => return Ok(answer),
            Err(e) => {
                eprintln!("quran-eg: saving for offline use: {url}: {e}");
                why = "quran.com can't be reached".into();
            }
        }
    }
    Err(End::Failed(why))
}

/// Asks for every need not kept, and keeps the answers (an answer that is not whole is asked for once more).
pub fn download(job: &Job, store: &Store, fetch: Fetch, needs: &[Need], ctx: &egui::Context) -> End {
    for (i, need) in needs.iter().enumerate() {
        if job.stopped() {
            return End::Stopped;
        }
        if store.fetched(&need.url).is_none() {
            let mut whole = None;
            for _ in 0..2 {
                match ask(job, fetch, &need.url) {
                    Ok((status, body)) if (need.keeps)(status, &body) => whole = Some(body),
                    Ok(_) => {}
                    Err(end) => return end,
                }
                if whole.is_some() || !job.wait(WAITS[0]) {
                    break;
                }
            }
            let Some(body) = whole else { return End::Failed("an answer from quran.com came back incomplete".into()) };
            if let Err(e) = store.write(&need.url, &body) {
                return End::Failed(format!("it could not be written ({e})"));
            }
            if !job.wait(PAUSE) {
                return End::Stopped;
            }
        }
        // A surah is saved once its last request is.
        if need.surah.is_some() && needs.get(i + 1).is_none_or(|next| next.surah != need.surah) {
            job.progress().done += 1;
            ctx.request_repaint();
        }
    }
    End::Done
}

/// Asks for every kept answer again, and keeps the new one where it holds what the old one did.
pub fn update(job: &Job, store: &Store, fetch: Fetch, ctx: &egui::Context) -> End {
    let kept = store.list();
    job.begin(Part::Answers, kept.len());
    for (url, _) in kept {
        if job.stopped() {
            return End::Stopped;
        }
        if let Some(old) = store.read(&url) {
            let (status, body) = match ask(job, fetch, &url) {
                Ok(answer) => answer,
                Err(end) => return end,
            };
            if (200..300).contains(&status)
                && holds(&old.body, &body)
                && let Err(e) = store.write(&url, &body)
            {
                return End::Failed(format!("it could not be written ({e})"));
            }
            if !job.wait(PAUSE) {
                return End::Stopped;
            }
        }
        job.progress().done += 1;
        ctx.request_repaint();
    }
    End::Done
}

/// Fetches the fonts the font cache (`dir`) lacks, each from the first of its bases that has it, as the font store
/// would; a font never changes, so one kept is not asked for again. Counts the page fonts.
pub fn fonts(job: &Job, dir: &Path, fetch: Fetch, files: &[FontFile], ctx: &egui::Context) -> End {
    job.begin(Part::Fonts, files.iter().filter(|f| f.page.is_some()).count());
    for file in files {
        if job.stopped() {
            return End::Stopped;
        }
        let kept = dir.join(cache_name(&file.path));
        if !kept.exists() {
            let bytes = match font(job, fetch, file) {
                Ok(bytes) => bytes,
                Err(end) => return end,
            };
            if let Err(e) = write_atomic(&kept, &bytes) {
                return End::Failed(format!("a font could not be written ({e})"));
            }
            if !job.wait(PAUSE) {
                return End::Stopped;
            }
        }
        if file.page.is_some() {
            job.progress().done += 1;
            ctx.request_repaint();
        }
    }
    End::Done
}

/// The font `file`, from the first of its bases that has it (a base that is a folder is read from the disk).
fn font(job: &Job, fetch: Fetch, file: &FontFile) -> Result<Vec<u8>, End> {
    let mut why = String::from("no font source is configured");
    for base in file.bases.iter() {
        let location = join(base, &file.path);
        let bytes = if base.contains("://") {
            match ask(job, fetch, &location) {
                Ok((200..=299, bytes)) => bytes,
                Ok((status, _)) => {
                    why = format!("a font is missing (HTTP {status})");
                    continue;
                }
                Err(End::Failed(e)) => {
                    why = e;
                    continue;
                }
                Err(end) => return Err(end),
            }
        } else {
            match std::fs::read(&location) {
                Ok(bytes) => bytes,
                Err(e) => {
                    why = format!("{location}: {e}");
                    continue;
                }
            }
        };
        if (file.good)(&bytes) {
            return Ok(bytes);
        }
        why = "a font file could not be read".into();
    }
    Err(End::Failed(why))
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::sync::Arc;

    use quran_core::api::Client;
    use quran_core::feed::VerseSource;
    use quran_core::fonts::FontSet;
    use quran_core::settings::Settings;

    use super::*;
    use crate::offline::answer::tests::surah;
    use crate::offline::copy::{need, survey_of};
    use crate::offline::job::Kind;
    use crate::offline::store::tests::{remove, scratch, scratch_dir};

    #[test]
    fn a_download_skips_what_is_kept_and_counts_surahs() {
        let (store, client, query) = (scratch("download"), Client::default(), Settings::default().verse_query());
        let surahs = [112, 113, 114].map(|c| need(VerseSource::Chapter { chapter: c, span: None }.call(&client, &query, 1), Some(c)));
        store.write(&surahs[1].url, &surah(113, 1, 0)).unwrap();
        let asked = Cell::new(0);
        let api = |url: &str| {
            asked.set(asked.get() + 1);
            let c = if url.contains("/by_chapter/112?") { 112 } else { 114 };
            Ok((200, surah(c, 1, 0)))
        };
        let job = Job::new(Kind::Download);
        assert_eq!(download(&job, &store, &api, &surahs, &egui::Context::default()), End::Done);
        let progress = job.progress().clone();
        assert_eq!((progress.done, progress.total, asked.get()), (3, 114, 2));
        let survey = survey_of(&store, &surahs, &scratch_dir("download-fonts"), &[]);
        assert_eq!((survey.surahs, survey.whole), (3, true));
        // An answer that is not whole is asked for twice, then the download stops there; so does a stop.
        let job = Job::new(Kind::Download);
        let short = |_: &str| Ok((200, surah(112, 1, 1)));
        let fresh = scratch("short");
        assert!(matches!(download(&job, &fresh, &short, &surahs, &egui::Context::default()), End::Failed(_)));
        assert!(fresh.list().is_empty());
        job.stop();
        assert_eq!(download(&job, &fresh, &short, &surahs, &egui::Context::default()), End::Stopped);
        remove(&store);
    }

    #[test]
    fn fonts_come_from_the_first_base_that_has_them_and_are_kept_once() {
        let (dir, folder) = (scratch_dir("fonts"), scratch_dir("fonts-folder"));
        // The first base has no such file; the second answers; a third is a folder on this computer.
        let bases = Arc::new(vec!["https://a.example".to_string(), "https://b.example".to_string(), folder.display().to_string()]);
        let good: fn(&[u8]) -> bool = |b| b.starts_with(b"font");
        let page = |p: u16| FontFile { path: FontSet::V4Colr.path(p), bases: bases.clone(), page: Some(p), good };
        let files = [page(1), page(2), page(3)];
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(cache_name(&files[0].path)), b"font kept").unwrap();
        std::fs::create_dir_all(folder.join("hafs/v4/colrv1/woff2")).unwrap();
        std::fs::write(folder.join(&files[2].path), b"font of the folder").unwrap();
        let asked = RefCell::new(Vec::new());
        let api = |url: &str| {
            asked.borrow_mut().push(url.to_string());
            let found = url.starts_with("https://b.example") && url.ends_with("p2.woff2");
            Ok(if found { (200, b"font two".to_vec()) } else { (404, Vec::new()) })
        };
        let job = Job::new(Kind::Download);
        assert_eq!(fonts(&job, &dir, &api, &files, &egui::Context::default()), End::Done);
        let read = |p: &FontFile| std::fs::read(dir.join(cache_name(&p.path))).unwrap();
        assert_eq!((read(&files[0]), read(&files[1]), read(&files[2])), (b"font kept".to_vec(), b"font two".to_vec(), b"font of the folder".to_vec()));
        // Page 1 was kept: not asked for; page 3 is in no web base, then read from the folder.
        assert_eq!(asked.borrow().len(), 4, "{:?}", asked.borrow());
        let progress = job.progress().clone();
        assert_eq!((progress.part, progress.done, progress.total), (Part::Fonts, 3, 3));
        // No base has it: the job stops, saying why.
        let job = Job::new(Kind::Download);
        let missing = [FontFile { path: "hafs/none.woff2".into(), bases, page: None, good }];
        assert!(matches!(fonts(&job, &dir, &api, &missing, &egui::Context::default()), End::Failed(why) if why.contains("hafs/none.woff2")));
        let _ = (std::fs::remove_dir_all(&dir), std::fs::remove_dir_all(&folder));
    }
}
