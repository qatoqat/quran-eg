//! The Quran reader without a GUI: everything that can be unit-tested.
//!
//! - `api`: typed blocking client for the Quran.com API v4.
//! - `http`: the blocking HTTP GET both the API client and the font loader use.
//! - `parse`: verse references (`2:255`, `2/1-5`), pagination math, slugs.
//! - `view`: what is on screen (list, surah, juz, page), the reading position
//!   and the saved location; where the header's toggle and prev/next lead.
//! - `feed`: paged verse loading state for a view, merged and checked.
//! - `integrity` + `reference`: checks that no verse or word goes missing,
//!   against compiled-in tables (verses per surah, words per verse, page starts).
//! - `mushaf_layout`: grouping of words into 15-line mushaf pages and lines.
//! - `glyph`: glyph vs Unicode-text choice per word (never nothing).
//! - `fonts`: which font files draw the mushaf and where they live.
//! - `nav`: navigation shortcuts typed into the search box (`2:255`, `page 3`, `kahf`).
//! - `html`, `highlight`, `verse_text`: API text reduced to safe segments, search
//!   highlights, and the plain text of a verse.
//! - `lists`: the surah and juz lists (filtering, lookups).
//! - `offline`: which answers are whole enough to keep for reading offline, and a
//!   mushaf page's or a juz's verses put together from the surahs' answers.
//! - `settings`: the settings model and its JSON persistence.

pub mod api;
pub mod feed;
pub mod fonts;
pub mod glyph;
pub mod highlight;
pub mod html;
#[cfg(not(target_arch = "wasm32"))]
pub mod http;
pub mod integrity;
pub mod lists;
pub mod mushaf_layout;
pub mod nav;
pub mod offline;
pub mod parse;
pub mod reference;
pub mod settings;
pub mod util;
pub mod verse_text;
pub mod view;
