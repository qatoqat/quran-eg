//! Font files: fetched at runtime (disk cache first, then each font base in
//! order), parsed off the UI thread, and dropped again when their page is far
//! from the viewport. A page whose font is not there is drawn as text.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use quran_core::fonts::{FALLBACK_PATH, FontSet, cache_name, join};
use quran_core::http::Http;
use quran_core::settings::storage::{APP_DIR, write_atomic};

use crate::arabic::{ArabicFont, Ornaments};
use crate::glyphs::{PageGlyphs, parse_page_font};
use crate::task;

/// Largest font file accepted (the biggest page font is well under 1 MB).
const MAX_FONT_BYTES: u64 = 8 * 1024 * 1024;

pub enum Status<T: ?Sized> {
    Loading,
    Ready(Arc<T>),
    /// (The reason was logged when it happened.)
    Failed,
}

impl<T: ?Sized> Status<T> {
    fn of(result: Result<Arc<T>, String>) -> Self {
        match result {
            Ok(font) => Status::Ready(font),
            Err(_) => Status::Failed,
        }
    }
}

enum Loaded {
    Page(u64, FontSet, u16, Result<Arc<dyn PageGlyphs>, String>),
    Fallback(u64, Result<Arc<ArabicFont>, String>),
    Ornaments(Result<Arc<Ornaments>, String>),
}

/// Where the surah banner's two ornament fonts come from (QUL, the Quranic
/// Universal Library). QUL lists no licence for them, so they are fetched, not bundled.
const ORNAMENT_BASE: &str = "https://static-cdn.tarteel.ai/qul/fonts";
const ORNAMENT_FRAME: &str = "common/quran-common.ttf";
const ORNAMENT_NAMES: &str = "surah-names/v4/surah-name-v4.ttf";

pub struct FontStore {
    ctx: egui::Context,
    bases: Arc<Vec<String>>,
    /// Bumped when the bases change, so results from the old ones are dropped.
    epoch: u64,
    cache_dir: Option<PathBuf>,
    pages: HashMap<(FontSet, u16), Status<dyn PageGlyphs>>,
    fallback: Option<Status<ArabicFont>>,
    ornaments: Option<Status<Ornaments>>,
    tx: Sender<Loaded>,
    rx: Receiver<Loaded>,
}

impl FontStore {
    /// The user's font cache: `<cache dir>/quran-eg/fonts`.
    pub fn user_cache() -> Option<PathBuf> {
        dirs::cache_dir().map(|d| d.join(APP_DIR).join("fonts"))
    }

    /// Fonts from `bases`, tried in order, kept in `cache_dir` once fetched.
    pub fn new(ctx: &egui::Context, bases: Vec<String>, cache_dir: Option<PathBuf>) -> Self {
        let (tx, rx) = channel();
        Self {
            ctx: ctx.clone(),
            bases: Arc::new(bases),
            epoch: 0,
            cache_dir,
            pages: HashMap::new(),
            fallback: None,
            ornaments: None,
            tx,
            rx,
        }
    }

    /// Take in the fonts that finished loading since the last frame.
    pub fn poll(&mut self) {
        while let Ok(loaded) = self.rx.try_recv() {
            match loaded {
                Loaded::Page(epoch, set, page, result) if epoch == self.epoch => {
                    if let Err(e) = &result {
                        eprintln!("quran-eg: font for page {page} ({}) could not be loaded: {e}", set.key());
                    }
                    // Only if it is still wanted (not dropped meanwhile).
                    if let Some(entry) = self.pages.get_mut(&(set, page)) {
                        *entry = Status::of(result);
                    }
                }
                Loaded::Fallback(epoch, result) if epoch == self.epoch => {
                    if let Err(e) = &result {
                        eprintln!("quran-eg: the fallback text font could not be loaded: {e}");
                    }
                    self.fallback = Some(Status::of(result));
                }
                Loaded::Ornaments(result) => {
                    if let Err(e) = &result {
                        eprintln!("quran-eg: the surah banner fonts could not be loaded (banners stay plain): {e}");
                    }
                    self.ornaments = Some(Status::of(result));
                }
                _ => {}
            }
        }
    }

    /// The surah banner's ornament fonts, if they are loaded; starts loading
    /// them otherwise. Until then (or if they can't be had) banners are plain.
    pub fn ornaments(&mut self) -> Option<Arc<Ornaments>> {
        let entry = self.ornaments.get_or_insert_with(|| {
            let cache_dir = self.cache_dir.clone();
            // Without a cache directory nothing may be fetched either (tests).
            let bases: Vec<String> = cache_dir.iter().map(|_| ORNAMENT_BASE.to_string()).collect();
            task::spawn(task::Pool::Net, &self.ctx, &self.tx, move || {
                let font = |path: &str| load(&bases, cache_dir.as_deref(), path, ArabicFont::from_bytes);
                let both = || Ornaments::verified(font(ORNAMENT_FRAME)?, font(ORNAMENT_NAMES)?).map(Arc::new);
                Loaded::Ornaments(task::guarded(both, died))
            });
            Status::Loading
        });
        match entry {
            Status::Ready(fonts) => Some(fonts.clone()),
            _ => None,
        }
    }

    /// The font of a page, if it is loaded; starts loading it otherwise.
    pub fn page(&mut self, set: FontSet, page: u16) -> Option<Arc<dyn PageGlyphs>> {
        let entry = self.pages.entry((set, page)).or_insert_with(|| {
            let (bases, cache_dir, epoch) = (self.bases.clone(), self.cache_dir.clone(), self.epoch);
            task::spawn(task::Pool::Net, &self.ctx, &self.tx, move || {
                let font = task::guarded(|| load(&bases, cache_dir.as_deref(), &set.path(page), parse_page_font), died);
                Loaded::Page(epoch, set, page, font)
            });
            Status::Loading
        });
        match entry {
            Status::Ready(font) => Some(font.clone()),
            _ => None,
        }
    }

    /// Loading this page's font was tried and failed (its words stay text).
    pub fn page_failed(&self, set: FontSet, page: u16) -> bool {
        matches!(self.pages.get(&(set, page)), Some(Status::Failed))
    }

    /// The font for text-mode words and Arabic labels, if it is loaded.
    pub fn fallback(&mut self) -> Option<Arc<ArabicFont>> {
        let entry = self.fallback.get_or_insert_with(|| {
            let (bases, cache_dir, epoch) = (self.bases.clone(), self.cache_dir.clone(), self.epoch);
            task::spawn(task::Pool::Net, &self.ctx, &self.tx, move || {
                let parse = |bytes: &[u8]| ArabicFont::from_bytes(bytes).map(Arc::new);
                let font = task::guarded(|| load(&bases, cache_dir.as_deref(), FALLBACK_PATH, parse), died);
                Loaded::Fallback(epoch, font)
            });
            Status::Loading
        });
        match entry {
            Status::Ready(font) => Some(font.clone()),
            _ => None,
        }
    }

    pub fn fallback_failed(&self) -> bool {
        matches!(self.fallback, Some(Status::Failed))
    }

    /// Free the loaded page fonts `keep` doesn't want (pages far from the viewport).
    pub fn retain_pages(&mut self, keep: impl Fn(FontSet, u16) -> bool) {
        self.pages.retain(|&(set, page), status| !matches!(status, Status::Ready(_)) || keep(set, page));
    }

    pub fn failed_pages(&self, set: FontSet) -> Vec<u16> {
        let mut pages: Vec<u16> = self
            .pages
            .iter()
            .filter(|((s, _), status)| *s == set && matches!(status, Status::Failed))
            .map(|((_, page), _)| *page)
            .collect();
        pages.sort_unstable();
        pages
    }

    /// Forget failures, so the fonts are requested again when next needed.
    pub fn retry_failed(&mut self) {
        self.pages.retain(|_, status| !matches!(status, Status::Failed));
        if self.fallback_failed() {
            self.fallback = None;
        }
        if matches!(self.ornaments, Some(Status::Failed)) {
            self.ornaments = None;
        }
    }

    /// Use other font bases (the custom font source changed): start over.
    pub fn set_bases(&mut self, bases: Vec<String>) {
        if *self.bases != bases {
            self.bases = Arc::new(bases);
            self.epoch += 1;
            self.pages.clear();
            self.fallback = None;
        }
    }

    pub fn loaded_pages(&self) -> usize {
        self.pages.values().filter(|s| matches!(s, Status::Ready(_))).count()
    }
}

/// The result of a font job that panicked.
fn died<T>() -> Result<T, String> {
    Err("the font file could not be read (internal error)".into())
}

fn http() -> &'static Http {
    static HTTP: std::sync::OnceLock<Http> = std::sync::OnceLock::new();
    HTTP.get_or_init(|| Http::new(Duration::from_secs(10), Duration::from_secs(30)))
}

fn fetch(base: &str, path: &str) -> Result<Vec<u8>, String> {
    let location = join(base, path);
    if !base.contains("://") {
        // A local copy of quran.com's `fonts/quran` folder.
        return std::fs::read(&location).map_err(|e| format!("{location}: {e}"));
    }
    match http().get(&location, "*/*") {
        Ok((200..=299, bytes)) if bytes.len() as u64 <= MAX_FONT_BYTES => Ok(bytes),
        Ok((200..=299, _)) => Err(format!("{location}: too large to be a font")),
        Ok((status, _)) => Err(format!("{location}: HTTP {status}")),
        Err(e) => Err(format!("{location}: {e}")),
    }
}

/// The font at `path`: from the cache, else from the first base that has a
/// file `parse` accepts (which is then cached). A cached file that no longer
/// parses is thrown away and fetched again.
fn load<T>(
    bases: &[String],
    cache_dir: Option<&Path>,
    path: &str,
    parse: impl Fn(&[u8]) -> Result<T, String>,
) -> Result<T, String> {
    let cached = cache_dir.map(|dir| dir.join(cache_name(path)));
    if let Some(file) = &cached
        && let Ok(bytes) = std::fs::read(file)
    {
        match parse(&bytes) {
            Ok(font) => return Ok(font),
            Err(_) => {
                let _ = std::fs::remove_file(file);
            }
        }
    }
    let mut last = String::from("no font source is configured");
    for base in bases {
        match fetch(base, path).and_then(|bytes| parse(&bytes).map(|font| (font, bytes))) {
            Ok((font, bytes)) => {
                if let Some(file) = &cached {
                    let _ = write_atomic(file, &bytes);
                }
                return Ok(font);
            }
            Err(e) => last = e,
        }
    }
    Err(last)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("quran-eg-fonts-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn parse(bytes: &[u8]) -> Result<String, String> {
        let text = String::from_utf8_lossy(bytes).into_owned();
        if text.starts_with("font") { Ok(text) } else { Err(format!("not a font: {text}")) }
    }

    #[test]
    fn bases_are_tried_in_order_and_the_result_is_cached() {
        let dir = scratch("order");
        let (first, second, cache) = (dir.join("first"), dir.join("second"), dir.join("cache"));
        std::fs::create_dir_all(second.join("hafs")).unwrap();
        std::fs::create_dir_all(first.join("hafs")).unwrap();
        // The first base has a broken file, the second the real one.
        std::fs::write(first.join("hafs/p1.woff2"), "broken").unwrap();
        std::fs::write(second.join("hafs/p1.woff2"), "font one").unwrap();
        let bases = [first.display().to_string(), second.display().to_string()];

        assert_eq!(load(&bases, Some(&cache), "hafs/p1.woff2", parse).as_deref(), Ok("font one"));
        let cached = cache.join(cache_name("hafs/p1.woff2"));
        assert_eq!(std::fs::read_to_string(&cached).unwrap(), "font one");

        // From now on the cache answers, even with no base reachable.
        assert_eq!(load(&[], Some(&cache), "hafs/p1.woff2", parse).as_deref(), Ok("font one"));

        // A cached file that no longer parses is replaced.
        std::fs::write(&cached, "rubbish").unwrap();
        assert_eq!(load(&bases, Some(&cache), "hafs/p1.woff2", parse).as_deref(), Ok("font one"));
        assert_eq!(std::fs::read_to_string(&cached).unwrap(), "font one");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn failure_names_the_last_source_and_caches_nothing() {
        let dir = scratch("fail");
        let cache = dir.join("cache");
        let bases = [dir.join("nowhere").display().to_string()];
        let err = load(&bases, Some(&cache), "hafs/p9.woff2", parse).unwrap_err();
        assert!(err.contains("nowhere/hafs/p9.woff2"), "{err}");
        assert!(!cache.join(cache_name("hafs/p9.woff2")).exists());
        assert!(load(&[], None, "x", parse).unwrap_err().contains("no font source"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
