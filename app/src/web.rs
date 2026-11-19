//! The browser's side of the web build: the address bar (the location lives in
//! the URL, as in quran-dx: the path is the view, the hash the reading
//! position, both under the site's base path), the history, and the tab's title.

use std::cell::Cell;

use wasm_bindgen::JsCast as _;
use wasm_bindgen::prelude::Closure;

thread_local! {
    /// Back or forward was used since the app last looked.
    static POPPED: Cell<bool> = const { Cell::new(false) };
}

fn window() -> web_sys::Window {
    web_sys::window().expect("a browser window")
}

/// The path the site is served under, ending in `/`: `/quran-eg/` on GitHub
/// Pages, from the page's `<base href>`.
pub fn base_path() -> String {
    let base = window().document().and_then(|d| d.base_uri().ok().flatten()).unwrap_or_default();
    // "https://host/quran-eg/" → "/quran-eg/"
    let path = base.split_once("://").and_then(|(_, rest)| rest.find('/').map(|slash| &rest[slash..])).unwrap_or("/");
    let path = path.split(['?', '#']).next().unwrap_or("/");
    match path.rfind('/') {
        Some(end) => path[..=end].to_string(),
        None => "/".to_string(),
    }
}

/// Where the address bar is, as a location of the app: `/2#2:142`, `/search?q=mercy`.
pub fn current() -> String {
    let location = window().location();
    let path = location.pathname().unwrap_or_default();
    let base = base_path();
    let below = path.strip_prefix(base.trim_end_matches('/')).unwrap_or(&path);
    let below = if below.starts_with('/') { below.to_string() } else { format!("/{below}") };
    format!("{below}{}{}", location.search().unwrap_or_default(), location.hash().unwrap_or_default())
}

fn url(location: &str) -> String {
    format!("{}{}", base_path(), location.trim_start_matches('/'))
}

/// A new entry in the history: the app went somewhere.
pub fn push(location: &str) {
    if let Ok(history) = window().history() {
        let _ = history.push_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some(&url(location)));
    }
}

/// The same entry, at another reading position.
pub fn replace(location: &str) {
    if let Ok(history) = window().history() {
        let _ = history.replace_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some(&url(location)));
    }
}

pub fn set_title(title: &str) {
    if let Some(document) = window().document() {
        document.set_title(title);
    }
}

/// Wake the app when back or forward is used.
pub fn listen(ctx: &egui::Context) {
    let ctx = ctx.clone();
    let on_pop = Closure::<dyn FnMut()>::new(move || {
        POPPED.set(true);
        ctx.request_repaint();
    });
    let _ = window().add_event_listener_with_callback("popstate", on_pop.as_ref().unchecked_ref());
    // Lives as long as the page.
    on_pop.forget();
}

/// Whether back or forward was used since the last call.
pub fn popped() -> bool {
    POPPED.replace(false)
}
