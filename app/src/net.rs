//! HTTP GET, off the frame: on a network thread natively (blocking, IPv4
//! first — see `quran_core::http`), with the browser's `fetch` on the web.
//! The answer goes to a callback; the UI is woken afterwards.

use std::sync::mpsc::{Receiver, channel};

use quran_core::api::{Call, Error};

/// The status and the body, or why there is neither.
pub type Fetched = Result<(u16, Vec<u8>), String>;

#[cfg(not(target_arch = "wasm32"))]
pub fn get(ctx: &egui::Context, url: String, accept: &'static str, done: impl FnOnce(Fetched) + Send + 'static) {
    use std::time::Duration;

    use quran_core::http::Http;

    static HTTP: std::sync::OnceLock<Http> = std::sync::OnceLock::new();
    crate::task::detach(crate::task::Pool::Net, ctx, move || {
        // Deadlines, so a hung connection turns into a visible error instead of an endless wait.
        let http = HTTP.get_or_init(|| Http::new(Duration::from_secs(15), Duration::from_secs(45)));
        done(http.get(&url, accept).map_err(|e| e.to_string()));
    });
}

#[cfg(target_arch = "wasm32")]
pub fn get(ctx: &egui::Context, url: String, accept: &'static str, done: impl FnOnce(Fetched) + Send + 'static) {
    let ctx = ctx.clone();
    let mut request = ehttp::Request::get(url);
    request.headers.insert("Accept", accept);
    ehttp::fetch(request, move |result| {
        done(result.map(|response| (response.status, response.bytes)));
        ctx.request_repaint();
    });
}

/// Send an API call; what `then` makes of its answer arrives on the returned channel.
pub fn call<T: 'static, U: Send + 'static>(
    ctx: &egui::Context,
    call: Call<T>,
    then: impl FnOnce(Result<T, Error>) -> U + Send + 'static,
) -> Receiver<U> {
    let (tx, rx) = channel();
    get(ctx, call.url.clone(), "application/json", move |fetched| {
        let answer = match fetched {
            Ok((status, body)) => call.read(status, &body),
            Err(e) => Err(Error::Http(e)),
        };
        // The receiver is gone when the view that asked has been closed.
        let _ = tx.send(then(answer));
    });
    rx
}
