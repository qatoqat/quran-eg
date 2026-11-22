//! Which listener is installed, shown under its folder in the settings: the model it runs (the first line of its
//! README.txt, or its folder's name) and the `qv` build (when it was written), each with the start of its md5,
//! so a run can be told apart from the next one. The files are tens of megabytes: they are hashed on a thread,
//! and again only once one of them was rewritten. Beside them, the models it can run: its `model/` and each
//! folder of its `models/`, by their READMEs.

use std::io::{ErrorKind, Read};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::time::SystemTime;

use md5::{Digest, Md5};

use crate::task::{self, Pool, Poll};

/// How often the files are looked at again while the settings show them (a new model may come meanwhile).
const LOOK_AGAIN: f64 = 2.0;
/// Lines of a model's README after its name that say what it is.
const ABOUT_LINES: usize = 2;

/// What the settings show of the installed listener.
#[derive(Clone, Debug, PartialEq)]
pub struct Lines {
    pub model: String,
    pub qv: String,
}

/// A model the listener can run.
#[derive(Clone, Debug, PartialEq)]
pub struct Model {
    /// Its name in the settings: a folder of `models/`, or empty for `model/`.
    pub name: String,
    /// The first line of its README.txt (or its folder's name), and the lines after it that say what it is.
    pub title: String,
    pub about: String,
}

/// A folder, the model's folder in it, and the files that tell them as last seen: each one's length and when it
/// was written (`None` if missing). A new one of any is read again.
type Key = (PathBuf, PathBuf, [Option<(u64, SystemTime)>; 3]);

#[derive(Default)]
pub struct Installed {
    /// The folders and their files' state the lines are for, or are being read for.
    key: Option<Key>,
    lines: Option<Lines>,
    reading: Option<Receiver<Lines>>,
    /// When the files were last looked at (UI time, s).
    looked: f64,
    /// The models of the folder, and when they were listed (UI time, s).
    models: Option<(PathBuf, f64, Vec<Model>)>,
}

impl Installed {
    /// The listener installed in `dir` and its model `model` (a [`quran_core::settings::Voice::model`]), read again
    /// when one of their files changed; `None` while it is first read.
    pub fn lines(&mut self, ctx: &egui::Context, dir: &Path, model: &str) -> Option<&Lines> {
        let now = ctx.input(|i| i.time);
        let model = dir.join(quran_core::settings::model_folder(model));
        let moved = self.key.as_ref().is_none_or(|k| k.0 != dir || k.1 != model);
        if moved || now - self.looked >= LOOK_AGAIN {
            self.looked = now;
            let files = [model.join("README.txt"), model.join("model.bpk"), dir.join("qv")];
            let key = (dir.to_path_buf(), model.clone(), files.map(|f| stamp(&f)));
            if self.key.as_ref() != Some(&key) {
                if moved {
                    self.lines = None;
                }
                self.key = Some(key);
                let dir = dir.to_path_buf();
                // A reading still under way for the files of before is dropped with its receiver.
                self.reading = Some(task::run(Pool::Draw, ctx, move || read(&dir, &model)));
            }
        }
        if let Some(rx) = &self.reading {
            match task::poll(rx) {
                Poll::Done(lines) => (self.lines, self.reading) = (Some(lines), None),
                Poll::Lost => self.reading = None,
                Poll::Pending => {}
            }
        }
        self.lines.as_ref()
    }

    /// The models the listener in `dir` can run, its own `model/` first; listed again every few seconds (their
    /// READMEs are small).
    pub fn models(&mut self, ctx: &egui::Context, dir: &Path) -> &[Model] {
        let now = ctx.input(|i| i.time);
        if self.models.as_ref().is_none_or(|(d, at, _)| d != dir || now - at >= LOOK_AGAIN) {
            self.models = Some((dir.to_path_buf(), now, models(dir)));
        }
        self.models.as_ref().map_or(&[], |m| &m.2)
    }
}

/// `model/`, then each folder of `models/` that holds a model, by name.
fn models(dir: &Path) -> Vec<Model> {
    let found = std::fs::read_dir(dir.join("models")).into_iter().flatten().flatten().filter_map(|entry| {
        let name = quran_core::settings::storage::sanitize_model(entry.file_name().to_str()?)?;
        entry.path().join("model.bpk").is_file().then(|| described(&entry.path(), name))
    });
    let mut found: Vec<Model> = found.collect();
    found.sort_by(|a, b| a.name.cmp(&b.name));
    std::iter::once(described(&dir.join("model"), String::new())).chain(found).collect()
}

/// A model by its README: its name, and what it is.
fn described(dir: &Path, name: String) -> Model {
    let readme = std::fs::read_to_string(dir.join("README.txt")).unwrap_or_default();
    let mut lines = readme.lines().map(str::trim).skip_while(|l| l.is_empty());
    let title = lines.next().map_or_else(|| folder_name(dir), str::to_owned);
    let about = lines.take(ABOUT_LINES).collect::<Vec<_>>().join("\n");
    Model { name, title, about }
}

fn stamp(path: &Path) -> Option<(u64, SystemTime)> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.len(), meta.modified().ok()?))
}

/// The listener in `dir` and its model in `model`, read from their files (they are hashed).
fn read(dir: &Path, model: &Path) -> Lines {
    let model = match md5_start(&model.join("model.bpk")) {
        Some(md5) => format!("{}, md5 {md5}", model_name(model)),
        None => "not installed".into(),
    };
    let qv = dir.join("qv");
    let qv = match md5_start(&qv) {
        Some(md5) => match std::fs::metadata(&qv).and_then(|m| m.modified()).ok().and_then(local) {
            Some(when) => format!("qv of {when}, md5 {md5}"),
            None => format!("qv, md5 {md5}"),
        },
        None => "not installed".into(),
    };
    Lines { model, qv }
}

/// The first line of the model's README.txt, which says which run it is; or its folder's name (a link's target).
fn model_name(dir: &Path) -> String {
    let readme = std::fs::read_to_string(dir.join("README.txt")).unwrap_or_default();
    match readme.lines().next().map(str::trim).filter(|l| !l.is_empty()) {
        Some(line) => line.to_owned(),
        None => folder_name(dir),
    }
}

/// A folder's name, by the name a link to it points at.
fn folder_name(dir: &Path) -> String {
    let dir = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    dir.file_name().map_or_else(|| dir.display().to_string(), |n| n.to_string_lossy().into_owned())
}

/// The first 8 hex digits of a file's md5, as `md5sum` prints it; `None` if it cannot be read.
fn md5_start(path: &Path) -> Option<String> {
    let mut file = std::fs::File::open(path).ok()?;
    let (mut md5, mut buf) = (Md5::new(), vec![0; 1 << 20]);
    loop {
        match file.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => md5.update(&buf[..n]),
            Err(e) if e.kind() == ErrorKind::Interrupted => {}
            Err(_) => return None,
        }
    }
    Some(md5.finalize()[..4].iter().map(|b| format!("{b:02x}")).collect())
}

/// `t` in local time, as `2026-11-21 12:00` (what `ls -l` shows of the file).
#[cfg(unix)]
fn local(t: SystemTime) -> Option<String> {
    let secs = libc::time_t::try_from(t.duration_since(SystemTime::UNIX_EPOCH).ok()?.as_secs()).ok()?;
    // SAFETY: `localtime_r` reads `secs` and writes only the `tm` it is given, for which all zeros is a valid value.
    let tm = unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&secs, &mut tm).is_null() {
            return None;
        }
        tm
    };
    Some(format!("{}-{:02}-{:02} {:02}:{:02}", tm.tm_year + 1900, tm.tm_mon + 1, tm.tm_mday, tm.tm_hour, tm.tm_min))
}

/// Without a local time zone at hand the time is left out.
#[cfg(not(unix))]
fn local(_: SystemTime) -> Option<String> {
    None
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("quran-eg-installed-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("model")).unwrap();
        dir
    }

    #[test]
    fn a_listener_is_told_by_its_readme_and_hashes() {
        let dir = scratch("readme");
        std::fs::write(dir.join("model/model.bpk"), "abc").unwrap();
        std::fs::write(dir.join("model/README.txt"), "  personal-p2 (installed 2026-11-21 ~20:00)\nmore\n").unwrap();
        std::fs::write(dir.join("qv"), "").unwrap();
        let lines = read(&dir, &dir.join("model"));
        assert_eq!(lines.model, "personal-p2 (installed 2026-11-21 ~20:00), md5 90015098");
        assert!(lines.qv.starts_with("qv of 2") && lines.qv.ends_with(", md5 d41d8cd9"), "{}", lines.qv);
        // No README: the model's folder, by the name a link to it points at.
        std::fs::remove_file(dir.join("model/README.txt")).unwrap();
        std::fs::rename(dir.join("model"), dir.join("model.p2")).unwrap();
        std::os::unix::fs::symlink("model.p2", dir.join("model")).unwrap();
        assert_eq!(read(&dir, &dir.join("model")).model, "model.p2, md5 90015098");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn missing_files_read_as_not_installed() {
        let dir = scratch("missing");
        std::fs::write(dir.join("model/README.txt"), "a run\n").unwrap();
        let lines = read(&dir, &dir.join("model"));
        assert_eq!((lines.model.as_str(), lines.qv.as_str()), ("not installed", "not installed"));
        assert_eq!(read(&dir.join("nowhere"), &dir.join("nowhere/model")), lines);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_files_are_read_again_only_once_one_changed() {
        let dir = scratch("again");
        std::fs::write(dir.join("model/model.bpk"), "abc").unwrap();
        let (ctx, mut installed) = (egui::Context::default(), Installed::default());
        let wait = |installed: &mut Installed| {
            for _ in 0..500 {
                if installed.reading.is_none() {
                    break;
                }
                installed.lines(&ctx, &dir, "");
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            installed.lines(&ctx, &dir, "").cloned().unwrap()
        };
        installed.lines(&ctx, &dir, "");
        assert_eq!(wait(&mut installed).model, "model, md5 90015098");
        installed.looked = -LOOK_AGAIN;
        installed.lines(&ctx, &dir, "");
        assert!(installed.reading.is_none(), "nothing changed, nothing is hashed");
        std::fs::write(dir.join("model/model.bpk"), "abcd").unwrap();
        installed.looked = -LOOK_AGAIN;
        installed.lines(&ctx, &dir, "");
        assert_eq!(wait(&mut installed).model, "model, md5 e2fc714c");
        // Another model chosen: its own line, read at once.
        std::fs::create_dir_all(dir.join("models/g-b")).unwrap();
        std::fs::write(dir.join("models/g-b/model.bpk"), "abc").unwrap();
        installed.lines(&ctx, &dir, "g-b");
        assert!(installed.lines.is_none() && installed.reading.is_some());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_models_are_the_installed_one_then_each_folder_of_models_by_its_readme() {
        let dir = scratch("models");
        std::fs::write(dir.join("model/model.bpk"), "abc").unwrap();
        std::fs::write(dir.join("model/README.txt"), "personal-p2 (installed)\nYour own.\n").unwrap();
        for (name, readme) in [("h-a", Some("\nGeneric H-A\nRound H, arm A.\nCautious.\nMore.\n")), ("g-b", None), ("empty", None)] {
            std::fs::create_dir_all(dir.join("models").join(name)).unwrap();
            if name != "empty" {
                std::fs::write(dir.join("models").join(name).join("model.bpk"), "x").unwrap();
            }
            if let Some(readme) = readme {
                std::fs::write(dir.join("models").join(name).join("README.txt"), readme).unwrap();
            }
        }
        std::fs::write(dir.join("models/stray.txt"), "not a model").unwrap();
        let list = models(&dir);
        let names: Vec<(&str, &str, &str)> = list.iter().map(|m| (m.name.as_str(), m.title.as_str(), m.about.as_str())).collect();
        assert_eq!(names, [("", "personal-p2 (installed)", "Your own."), ("g-b", "g-b", ""), ("h-a", "Generic H-A", "Round H, arm A.\nCautious.")]);
        // No models folder: the installed one alone, even when missing.
        assert_eq!(models(&dir.join("nowhere")).iter().map(|m| m.name.as_str()).collect::<Vec<_>>(), [""]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn local_time_reads_as_a_date_and_a_minute() {
        let t = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(181 * 86_400);
        let s = local(t).unwrap();
        assert!(s.starts_with("1970-0") && s.len() == 16 && &s[10..11] == " " && &s[13..14] == ":", "{s}");
    }
}
