//! Font files: fetched at runtime (disk cache first, then each font base in
//! order), parsed off the UI thread, and dropped again when their page is far
//! from the viewport. A page whose font is not there is drawn as text.
//!
//! In a browser there is no disk cache here — the browser's own HTTP cache does
//! that — and a font base that is a path is a path on the site.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

use quran_core::fonts::{FALLBACK_PATH, FontSet, join};

use crate::arabic::{ArabicFont, Ornaments};
use crate::glyphs::{PageGlyphs, parse_page_font};
use crate::task;

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
    /// The user's font cache: `<cache dir>/quran-eg/fonts` (none in a browser).
    pub fn user_cache() -> Option<PathBuf> {
        #[cfg(not(target_arch = "wasm32"))]
        return dirs::cache_dir().map(|d| d.join(quran_core::settings::storage::APP_DIR).join("fonts"));
        #[cfg(target_arch = "wasm32")]
        None
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
            // (Tests have no font source at all, and must not fetch these either.)
            let bases = if self.bases.is_empty() { Vec::new() } else { vec![ORNAMENT_BASE.to_string()] };
            let source = Source { ctx: self.ctx.clone(), bases: Arc::new(bases), cache_dir: self.cache_dir.clone() };
            let (tx, names_source) = (self.tx.clone(), source.clone());
            // The frame, then the names, then: do they draw the banner?
            load(source, ORNAMENT_FRAME.into(), ArabicFont::from_bytes, move |frame| match frame {
                Err(e) => drop(tx.send(Loaded::Ornaments(Err(e)))),
                Ok(frame) => load(names_source, ORNAMENT_NAMES.into(), ArabicFont::from_bytes, move |names| {
                    let both = names.and_then(|names| task::guarded(|| Ornaments::verified(frame, names).map(Arc::new), died));
                    let _ = tx.send(Loaded::Ornaments(both));
                }),
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
            let (tx, epoch) = (self.tx.clone(), self.epoch);
            let source = Source { ctx: self.ctx.clone(), bases: self.bases.clone(), cache_dir: self.cache_dir.clone() };
            load(source, set.path(page), parse_page_font, move |font| {
                let _ = tx.send(Loaded::Page(epoch, set, page, font));
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
            let (tx, epoch) = (self.tx.clone(), self.epoch);
            let source = Source { ctx: self.ctx.clone(), bases: self.bases.clone(), cache_dir: self.cache_dir.clone() };
            load(source, FALLBACK_PATH.into(), |bytes| ArabicFont::from_bytes(bytes).map(Arc::new), move |font| {
                let _ = tx.send(Loaded::Fallback(epoch, font));
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

/// Where fonts are looked for.
#[derive(Clone)]
struct Source {
    ctx: egui::Context,
    bases: Arc<Vec<String>>,
    /// (Unused in a browser, whose HTTP cache does this.)
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    cache_dir: Option<PathBuf>,
}

/// Largest font file accepted (the biggest page font is well under 1 MB).
const MAX_FONT_BYTES: usize = 8 * 1024 * 1024;

/// Get the font at `path` and hand it to `done`: from the cache, else from the
/// first base that has a file `parse` accepts (which is then cached). A cached
/// file that no longer parses is thrown away and fetched again.
fn load<T: Send + 'static>(
    source: Source,
    path: String,
    parse: fn(&[u8]) -> Result<T, String>,
    done: impl FnOnce(Result<T, String>) + Send + 'static,
) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        // Reading and parsing the cached file is work for a network thread, not for a frame.
        let ctx = source.ctx.clone();
        task::detach(task::Pool::Net, &ctx, move || {
            if let Some(file) = source.cache_dir.as_ref().map(|dir| dir.join(quran_core::fonts::cache_name(&path)))
                && let Ok(bytes) = std::fs::read(&file)
            {
                match task::guarded(|| parse(&bytes), died) {
                    Ok(font) => return done(Ok(font)),
                    Err(_) => drop(std::fs::remove_file(&file)),
                }
            }
            from_base(source, 0, path, parse, done, "no font source is configured".into());
        });
    }
    #[cfg(target_arch = "wasm32")]
    from_base(source, 0, path, parse, done, "no font source is configured".into());
}

/// Try base `index`, and the next one if it has no usable file.
fn from_base<T: Send + 'static>(
    source: Source,
    index: usize,
    path: String,
    parse: fn(&[u8]) -> Result<T, String>,
    done: impl FnOnce(Result<T, String>) + Send + 'static,
    last_error: String,
) {
    let Some(base) = source.bases.get(index).cloned() else { return done(Err(last_error)) };
    let location = join(&base, &path);
    let ctx = source.ctx.clone();
    let named = location.clone();
    let take = move |fetched: Result<Vec<u8>, String>| {
        let parsed = fetched.and_then(|bytes| task::guarded(|| parse(&bytes), died).map(|font| (font, bytes)));
        match parsed {
            Ok((font, _bytes)) => {
                #[cfg(not(target_arch = "wasm32"))]
                if let Some(dir) = &source.cache_dir {
                    let _ = quran_core::settings::storage::write_atomic(&dir.join(quran_core::fonts::cache_name(&path)), &_bytes);
                }
                done(Ok(font));
            }
            Err(e) => from_base(source, index + 1, path, parse, done, format!("{named}: {e}")),
        }
    };
    // A base that is a path: a local copy of quran.com's `fonts/quran` folder
    // (in a browser, a path on the site, which is fetched like any other).
    #[cfg(not(target_arch = "wasm32"))]
    if !base.contains("://") {
        return take(std::fs::read(&location).map_err(|e| e.to_string()));
    }
    crate::net::get(&ctx, location, "*/*", move |fetched| {
        take(match fetched {
            Ok((200..=299, bytes)) if bytes.len() <= MAX_FONT_BYTES => Ok(bytes),
            Ok((200..=299, _)) => Err("too large to be a font".into()),
            Ok((status, _)) => Err(format!("HTTP {status}")),
            Err(e) => Err(e),
        })
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use quran_core::fonts::cache_name;

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

    /// Load `path` and wait for the answer.
    fn fetch(bases: &[String], cache: Option<&PathBuf>, path: &str) -> Result<String, String> {
        let source = Source { ctx: egui::Context::default(), bases: Arc::new(bases.to_vec()), cache_dir: cache.cloned() };
        let (tx, rx) = channel();
        load(source, path.into(), parse, move |font| drop(tx.send(font)));
        rx.recv_timeout(std::time::Duration::from_secs(10)).expect("the font job answers")
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

        assert_eq!(fetch(&bases, Some(&cache), "hafs/p1.woff2").as_deref(), Ok("font one"));
        let cached = cache.join(cache_name("hafs/p1.woff2"));
        assert_eq!(std::fs::read_to_string(&cached).unwrap(), "font one");

        // From now on the cache answers, even with no base reachable.
        assert_eq!(fetch(&[], Some(&cache), "hafs/p1.woff2").as_deref(), Ok("font one"));

        // A cached file that no longer parses is replaced.
        std::fs::write(&cached, "rubbish").unwrap();
        assert_eq!(fetch(&bases, Some(&cache), "hafs/p1.woff2").as_deref(), Ok("font one"));
        assert_eq!(std::fs::read_to_string(&cached).unwrap(), "font one");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn failure_names_the_last_source_and_caches_nothing() {
        let dir = scratch("fail");
        let cache = dir.join("cache");
        let bases = [dir.join("nowhere").display().to_string()];
        let err = fetch(&bases, Some(&cache), "hafs/p9.woff2").unwrap_err();
        assert!(err.contains("nowhere/hafs/p9.woff2"), "{err}");
        assert!(!cache.join(cache_name("hafs/p9.woff2")).exists());
        assert!(fetch(&[], None, "x").unwrap_err().contains("no font source"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
