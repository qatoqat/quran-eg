//! Timings for looking at performance: when the first frame, the first verses
//! and each page's sheet were ready, and how long frames take.
//!
//! Natively they are printed with `QURAN_EG_STATS` set (and cost one check of a
//! flag otherwise). In a browser the milestones are `performance.mark`s
//! (`quran-eg:first-frame`, `:first-verses`, `:first-sheet`), readable from the
//! developer tools.

use web_time::Instant;

fn start() -> Instant {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    *START.get_or_init(Instant::now)
}

/// Whether timings are wanted.
pub fn on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        start();
        cfg!(target_arch = "wasm32") || std::env::var_os("QURAN_EG_STATS").is_some()
    })
}

/// Record a milestone, the first time it is reached.
pub fn milestone(name: &'static str) {
    static SEEN: std::sync::Mutex<Vec<&'static str>> = std::sync::Mutex::new(Vec::new());
    if !on() {
        return;
    }
    let Ok(mut seen) = SEEN.lock() else { return };
    if seen.contains(&name) {
        return;
    }
    seen.push(name);
    #[cfg(not(target_arch = "wasm32"))]
    {
        // (The wall clock too, to time the start-up from outside the process.)
        let unix = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis());
        eprintln!("stats: +{:.0} ms {name} (unix {unix})", start().elapsed().as_secs_f64() * 1000.0);
    }
    #[cfg(target_arch = "wasm32")]
    if let Some(performance) = web_sys::window().and_then(|w| w.performance()) {
        let _ = performance.mark(&format!("quran-eg:{name}"));
    }
}

/// Print a timing (native builds, with `QURAN_EG_STATS`).
pub fn note(detail: impl FnOnce() -> String) {
    #[cfg(not(target_arch = "wasm32"))]
    if on() {
        eprintln!("stats: +{:.0} ms {}", start().elapsed().as_secs_f64() * 1000.0, detail());
    }
    #[cfg(target_arch = "wasm32")]
    let _ = detail;
}

/// Frame times, printed once a second while there are frames (native builds,
/// with `QURAN_EG_STATS`).
#[derive(Default)]
pub struct Frames {
    count: u32,
    total: f64,
    longest: f64,
    since: Option<Instant>,
    /// What asked for the frames since (egui's record of the calls, by file and line), and how often.
    causes: std::collections::BTreeMap<String, u32>,
}

impl Frames {
    /// Time one frame, and note what asked for it.
    pub fn time<R>(&mut self, ctx: &egui::Context, frame: impl FnOnce() -> R) -> R {
        if cfg!(target_arch = "wasm32") || !on() {
            return frame();
        }
        for cause in ctx.repaint_causes() {
            *self.causes.entry(format!("{}:{}", cause.file.rsplit('/').next().unwrap_or(cause.file), cause.line)).or_default() += 1;
        }
        let began = Instant::now();
        let result = frame();
        let ms = began.elapsed().as_secs_f64() * 1000.0;
        self.count += 1;
        self.total += ms;
        self.longest = self.longest.max(ms);
        let since = *self.since.get_or_insert(began);
        if since.elapsed().as_secs_f64() >= 1.0 {
            let (count, average) = (self.count, self.total / f64::from(self.count));
            let mut causes: Vec<_> = self.causes.iter().collect();
            causes.sort_by(|a, b| b.1.cmp(a.1));
            let causes: Vec<String> = causes.iter().take(4).map(|(at, n)| format!("{at} ×{n}")).collect();
            note(|| format!("frames: {count} in the last second, {average:.2} ms each on average, longest {:.2} ms; asked by {}", self.longest, causes.join(", ")));
            *self = Self::default();
        }
        result
    }
}
