//! Typed blocking client for the public Quran.com API v4 (<https://api.quran.com/api/v4>).
//!
//! The base URL is configurable because the public API may move or require auth.
//! Requests block: the app calls them from worker threads (see `client`).

pub mod client;
pub mod query;
pub mod types;

pub use client::{Client, Error, parse_json};
pub use query::VerseQuery;
pub use types::*;

/// Default public endpoint.
pub const DEFAULT_BASE_URL: &str = "https://api.quran.com/api/v4";
