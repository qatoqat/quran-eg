//! The Noble Quran: a native reader for the 15-line Madani mushaf.
//!
//! - `app`: state, navigation, persistence of settings and location.
//! - `header`, `lists`, `settings_panel`: the chrome and the two lists.
//! - `reader`: the mushaf views (loading, layout, scrolling, drawing).
//! - `fonts`: fetching, caching and freeing font files.
//! - `sheet`: a page's words rasterised into one texture.
//! - `glyphs` / `arabic`: QCF page glyphs / shaped fallback text.
//! - `theme`, `kit`: colours and small widgets.
//! - `task`, `net`: work and requests off the frame (threads natively, a queue and `fetch` in a browser).
//! - `web` (web build only): the address bar, history and tab title.
//! - `recorder`, `voice` (desktop only): recording the recitation, and the voice pointer that follows it.

mod app;
mod arabic;
mod fonts;
mod glyphs;
mod header;
mod kit;
mod lists;
mod net;
mod pointer;
mod reader;
#[cfg(not(target_arch = "wasm32"))]
mod recorder;
mod remote;
mod search;
mod settings_panel;
mod sheet;
mod stats;
mod task;
mod theme;
#[cfg(not(target_arch = "wasm32"))]
mod voice;

#[cfg(target_arch = "wasm32")]
mod web;

#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result {
    let viewport = egui::ViewportBuilder::default()
        .with_title("The Noble Quran")
        .with_app_id("quran-eg")
        .with_inner_size([1100.0, 860.0])
        .with_min_inner_size([420.0, 320.0]);
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    eframe::run_native("quran-eg", options, Box::new(|cc| Ok(Box::new(app::App::new(cc)))))
}

/// The web build: the app runs in the page's canvas (see `web/index.html`).
#[cfg(target_arch = "wasm32")]
fn main() {
    use wasm_bindgen::JsCast as _;

    wasm_bindgen_futures::spawn_local(async {
        let document = web_sys::window().and_then(|w| w.document()).expect("a document");
        let canvas = document
            .get_element_by_id("app")
            .and_then(|e| e.dyn_into::<web_sys::HtmlCanvasElement>().ok())
            .expect("the page has a canvas with id `app`");
        let started = eframe::WebRunner::new()
            .start(canvas, eframe::WebOptions::default(), Box::new(|cc| Ok(Box::new(app::App::new(cc)))))
            .await;
        // The page shows "Loading…" until the app draws; if it can't start, it says why.
        if let Some(loading) = document.get_element_by_id("loading") {
            match started {
                Ok(()) => loading.remove(),
                Err(e) => loading.set_text_content(Some(&format!("The app could not start: {e:?}"))),
            }
        }
    });
}
