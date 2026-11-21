//! HTTP GET, off the frame: on a network thread natively (blocking, IPv4
//! first — see `quran_core::http`), with the browser's `fetch` on the web.
//! The answer goes to a callback; the UI is woken afterwards. Natively, the
//! API's answers come from the copy kept for reading offline when it has them
//! (see `offline`).

use std::sync::mpsc::{Receiver, channel};

use quran_core::api::{Call, Client, Error, VerseQuery, VersesResponse};
use quran_core::feed::VerseSource;

/// The status and the body, or why there is neither.
pub type Fetched = Result<(u16, Vec<u8>), String>;

/// The blocking HTTP client the network threads share.
#[cfg(not(target_arch = "wasm32"))]
pub fn http() -> &'static quran_core::http::Http {
    use std::time::Duration;

    static HTTP: std::sync::OnceLock<quran_core::http::Http> = std::sync::OnceLock::new();
    // Deadlines, so a hung connection turns into a visible error instead of an endless wait.
    HTTP.get_or_init(|| quran_core::http::Http::new(Duration::from_secs(15), Duration::from_secs(45)))
}

#[cfg(not(target_arch = "wasm32"))]
pub fn get(ctx: &egui::Context, url: String, accept: &'static str, done: impl FnOnce(Fetched) + Send + 'static) {
    crate::task::detach(crate::task::Pool::Net, ctx, move || done(http().get(&url, accept).map_err(|e| e.to_string())));
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
    // (The receiver is gone when the view that asked has been closed.)
    #[cfg(not(target_arch = "wasm32"))]
    crate::task::detach(crate::task::Pool::Net, ctx, move || crate::offline::answer(&call, |answer| drop(tx.send(then(answer)))));
    #[cfg(target_arch = "wasm32")]
    get(ctx, call.url.clone(), "application/json", move |fetched| {
        let answer = match fetched {
            Ok((status, body)) => call.read(status, &body),
            Err(e) => Err(Error::Http(e)),
        };
        let _ = tx.send(then(answer));
    });
    rx
}

/// Request `page` of a view's verses, as [`call`]. Natively, a mushaf page or a juz not kept as asked for is put
/// together from the surahs kept.
pub fn verses<U: Send + 'static>(
    ctx: &egui::Context,
    client: &Client,
    source: VerseSource,
    query: &VerseQuery,
    page: u32,
    then: impl FnOnce(Result<VersesResponse, Error>) -> U + Send + 'static,
) -> Receiver<U> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let (tx, rx) = channel();
        let (client, query) = (client.clone(), query.clone());
        crate::task::detach(crate::task::Pool::Net, ctx, move || {
            crate::offline::verses(&client, source, &query, page, |answer| drop(tx.send(then(answer))));
        });
        rx
    }
    #[cfg(target_arch = "wasm32")]
    call(ctx, source.call(client, query, page), then)
}
