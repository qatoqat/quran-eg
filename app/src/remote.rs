//! Lists fetched once from the API (surahs, ajzāʾ, translations), with retry.

use std::sync::mpsc::Receiver;

use quran_core::api::Call;

use crate::task::{self, Poll};

/// After a failed request for something the app needs in the background
/// (the lists' names, verse 1:1), it is tried again this many seconds later.
pub const RETRY_AFTER: f64 = 15.0;

/// A list fetched once from the API (surahs, ajzāʾ), with retry.
pub struct Remote<T> {
    pub items: Vec<T>,
    job: Option<Receiver<Result<Vec<T>, String>>>,
    pub error: Option<String>,
    /// When the last request failed.
    failed_at: f64,
}

impl<T: Send + 'static> Remote<T> {
    pub fn new() -> Self {
        Self { items: Vec::new(), job: None, error: None, failed_at: 0.0 }
    }

    pub fn loading(&self) -> bool {
        self.job.is_some()
    }

    pub fn poll(&mut self, now: f64) {
        let result = match self.job.as_ref().map(task::poll) {
            Some(Poll::Done(result)) => result,
            Some(Poll::Lost) => Err("the request ended unexpectedly".into()),
            Some(Poll::Pending) | None => return,
        };
        self.job = None;
        match result {
            Ok(items) => self.items = items,
            Err(e) => {
                self.error = Some(e);
                self.failed_at = now;
            }
        }
    }

    /// The last request failed long enough ago to try again.
    pub fn due(&self, now: f64) -> bool {
        self.error.is_some() && self.job.is_none() && now - self.failed_at >= RETRY_AFTER
    }

    /// Ask for the list (unless it is there or on its way); `tidy` puts the answer in order.
    pub fn load(&mut self, ctx: &egui::Context, call: Call<Vec<T>>, tidy: fn(Vec<T>) -> Vec<T>) {
        if self.items.is_empty() && self.job.is_none() {
            // (The error stays on screen until the new attempt has an answer.)
            self.job = Some(crate::net::call(ctx, call, move |answer| answer.map(tidy).map_err(|e| e.to_string())));
        }
    }
}
