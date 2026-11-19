//! The Noble Quran: a native reader for the 15-line Madani mushaf.
//!
//! - `app`: state, navigation, persistence of settings and location.
//! - `header`, `lists`, `settings_panel`: the chrome and the two lists.
//! - `reader`: the mushaf views (loading, layout, scrolling, drawing).
//! - `fonts`: fetching, caching and freeing font files.
//! - `sheet`: a page's words rasterised into one texture.
//! - `glyphs` / `arabic`: QCF page glyphs / shaped fallback text.
//! - `theme`, `kit`: colours and small widgets.
//! - `task`: worker threads.

mod app;
mod arabic;
mod fonts;
mod glyphs;
mod header;
mod kit;
mod lists;
mod pointer;
mod reader;
mod remote;
mod search;
mod settings_panel;
mod sheet;
mod task;
mod theme;

fn main() -> eframe::Result {
    let viewport = egui::ViewportBuilder::default()
        .with_title("The Noble Quran")
        .with_app_id("quran-eg")
        .with_inner_size([1100.0, 860.0])
        .with_min_inner_size([420.0, 320.0]);
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    eframe::run_native("quran-eg", options, Box::new(|cc| Ok(Box::new(app::App::new(cc)))))
}
