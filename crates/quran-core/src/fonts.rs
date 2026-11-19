//! Which font files draw the mushaf, and where they live below a font base URL
//! (quran.com's `fonts/quran` layout) and in the on-disk cache.

use crate::settings::MushafFont;

/// Unicode Hafs face for text-mode words. It expects the QPC text encoding
/// (`qpc_uthmani_hafs`), not plain `text_uthmani`.
pub const FALLBACK_PATH: &str = "hafs/uthmanic_hafs/UthmanicHafs1Ver18.woff2";

/// The font files that draw a [`MushafFont`] choice.
///
/// quran-dx also knows quran.com's OpenType-SVG files (`v4/ot-svg/{light,dark}`),
/// which exist for WebKit: it can't switch COLRv1 palettes. The rasteriser here
/// can, so the COLRv1 files serve both light and dark.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FontSet {
    /// QCF V2, single colour.
    V2,
    /// QCF V4 tajweed as COLRv1 (palette 0 light, 1 dark).
    V4Colr,
}

impl FontSet {
    pub fn of(font: MushafFont) -> Self {
        match font {
            MushafFont::MadaniV2 => FontSet::V2,
            MushafFont::TajweedV4 => FontSet::V4Colr,
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            FontSet::V2 => "v2",
            FontSet::V4Colr => "v4",
        }
    }

    /// Whether the font carries its own colours (the theme only picks the palette).
    pub fn coloured(self) -> bool {
        self == FontSet::V4Colr
    }

    /// Path of a page's font file below a font base URL.
    pub fn path(self, page: u16) -> String {
        let dir = match self {
            FontSet::V2 => "v2",
            FontSet::V4Colr => "v4/colrv1",
        };
        format!("hafs/{dir}/woff2/p{page}.woff2")
    }
}

/// File name of a font in the cache directory: its path below the base with the
/// separators flattened, so files of different sets can't collide.
pub fn cache_name(path: &str) -> String {
    path.chars().map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' { c } else { '_' }).collect()
}

/// Full URL (or local path, for a base that is a directory) of `path` below `base`.
pub fn join(base: &str, path: &str) -> String {
    format!("{}/{}", base.trim_end_matches('/'), path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_follow_quran_com() {
        assert_eq!(FontSet::V2.path(3), "hafs/v2/woff2/p3.woff2");
        assert_eq!(FontSet::V4Colr.path(604), "hafs/v4/colrv1/woff2/p604.woff2");
        assert_eq!(FontSet::of(MushafFont::TajweedV4), FontSet::V4Colr);
        assert_eq!(FontSet::of(MushafFont::MadaniV2), FontSet::V2);
        assert!(FontSet::V4Colr.coloured() && !FontSet::V2.coloured());
        assert_eq!(
            join("https://quran.com/fonts/quran/", &FontSet::V2.path(1)),
            "https://quran.com/fonts/quran/hafs/v2/woff2/p1.woff2"
        );
    }

    #[test]
    fn cache_names_are_flat_and_distinct() {
        let (a, b) = (cache_name(&FontSet::V2.path(12)), cache_name(&FontSet::V4Colr.path(12)));
        assert_eq!(a, "hafs_v2_woff2_p12.woff2");
        assert_eq!(b, "hafs_v4_colrv1_woff2_p12.woff2");
        assert_eq!(cache_name("../../etc/passwd"), ".._.._etc_passwd");
        assert_eq!(cache_name(FALLBACK_PATH), "hafs_uthmanic_hafs_UthmanicHafs1Ver18.woff2");
    }
}
