use std::collections::HashSet;

use super::*;

const V2: &[u8] = include_bytes!("../tests/fixtures/v2_p1.woff2");
const V4: &[u8] = include_bytes!("../tests/fixtures/v4_colr_p1.woff2");

/// First word of page 1 (بِسْمِ).
const WORD: char = '\u{FC41}';
/// A word with tajweed colouring in V4 (ٱلرَّحْمَٰنِ).
const TAJWEED_WORD: char = '\u{FC43}';
/// End-of-ayah marker of 1:1.
const MARKER: char = '\u{FC45}';
/// One past page 1's last glyph.
const MISSING: char = '\u{FC65}';

const BLACK: [u8; 4] = [0, 0, 0, 255];

fn v2() -> PageFont {
    PageFont::from_bytes(V2).unwrap()
}

fn v4() -> PageFont {
    PageFont::from_bytes(V4).unwrap()
}

fn opaque_colors(image: &GlyphImage) -> HashSet<[u8; 3]> {
    image
        .rgba
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|p| p[3] == 255)
        .map(|p| [p[0], p[1], p[2]])
        .collect()
}

fn assert_well_formed(image: &GlyphImage) {
    assert_eq!(
        image.rgba.len(),
        image.width as usize * image.height as usize * 4
    );
    for p in image.rgba.as_chunks::<4>().0.iter() {
        assert!(
            p[0] <= p[3] && p[1] <= p[3] && p[2] <= p[3],
            "not premultiplied: {p:?}"
        );
    }
    if image.width > 0 {
        let (w, h) = (image.width as usize, image.height as usize);
        let alpha = |x: usize, y: usize| image.rgba[(y * w + x) * 4 + 3];
        assert!(
            (0..w).any(|x| alpha(x, 0) != 0) && (0..w).any(|x| alpha(x, h - 1) != 0),
            "not cropped"
        );
        assert!(
            (0..h).any(|y| alpha(0, y) != 0) && (0..h).any(|y| alpha(w - 1, y) != 0),
            "not cropped"
        );
    }
}

#[test]
fn page_font_is_send_and_sync() {
    fn check<T: Send + Sync>() {}
    check::<PageFont>();
}

#[test]
fn decodes_woff2_and_plain_sfnt_alike() {
    let sfnt = wuff::decompress_woff2(V4).unwrap();
    assert!(sfnt.len() > V4.len());
    let (packed, plain) = (v4(), PageFont::from_bytes(&sfnt).unwrap());
    assert_eq!(packed.chars(), plain.chars());
    assert_eq!(packed.metrics_em(), plain.metrics_em());
    assert_eq!(
        packed.rasterize(MARKER, 48.0, Palette::Light, BLACK),
        plain.rasterize(MARKER, 48.0, Palette::Light, BLACK)
    );
}

#[test]
fn reports_font_kind_and_metrics() {
    let (v2, v4) = (v2(), v4());
    assert!(!v2.is_color() && v4.is_color());
    assert_eq!((v2.palette_count(), v4.palette_count()), (0, 6));
    let (ascent, descent) = v2.metrics_em();
    assert!((ascent - 2809.0 / 2500.0).abs() < 1e-6 && (descent - 1301.0 / 2500.0).abs() < 1e-6);
    let (ascent, descent) = v4.metrics_em();
    assert!(ascent > 1.0 && ascent < 2.0 && descent > 0.5 && descent < 1.5);
}

#[test]
fn knows_which_glyphs_exist() {
    for font in [v2(), v4()] {
        assert!(font.has_glyph(WORD) && font.has_glyph(MARKER));
        assert!(!font.has_glyph(MISSING) && !font.has_glyph('A') && !font.has_glyph('\u{0628}'));
        assert!(font.advance_em(WORD).is_some_and(|a| a > 0.5 && a < 5.0));
        assert_eq!(font.advance_em(MISSING), None);
        assert!(
            font.rasterize(MISSING, 48.0, Palette::Light, BLACK)
                .is_none()
        );
        let words: Vec<char> = font
            .chars()
            .into_iter()
            .filter(|c| font.has_glyph(*c))
            .collect();
        assert_eq!(words, ('\u{FC41}'..='\u{FC64}').collect::<Vec<_>>());
    }
}

#[test]
fn blank_glyph_has_advance_but_no_image() {
    let font = v4();
    assert!(font.chars().contains(&' ') && !font.has_glyph(' '));
    let space = font.rasterize(' ', 48.0, Palette::Light, BLACK).unwrap();
    assert_eq!((space.width, space.height, space.rgba.len()), (0, 0, 0));
    assert!(space.advance > 0.0);
}

#[test]
fn rasterizes_words_with_plausible_geometry() {
    for font in [v2(), v4()] {
        for size in [30.0, 64.0, 90.0] {
            for ch in font.chars().into_iter().filter(|c| font.has_glyph(*c)) {
                let image = font.rasterize(ch, size, Palette::Light, BLACK).unwrap();
                assert_well_formed(&image);
                assert!(
                    image.width as f32 > size * 0.2 && (image.width as f32) < size * 5.0,
                    "{ch:?} {}",
                    image.width
                );
                assert!(
                    image.height as f32 > size * 0.2 && (image.height as f32) < size * 3.0,
                    "{ch:?} {}",
                    image.height
                );
                assert_eq!(image.advance, font.advance_em(ch).unwrap() * size);
                // The ink sits around the baseline and roughly inside the advance.
                assert!(image.top > 0.0 && image.top < size * 2.0);
                assert!(
                    image.left > -size * 2.0
                        && image.left + (image.width as f32) < image.advance + size * 2.0
                );
                assert!(
                    image.rgba.as_chunks::<4>().0.iter().any(|p| p[3] > 200),
                    "{ch:?} has no solid ink"
                );
            }
        }
    }
}

#[test]
fn geometry_scales_with_size() {
    for font in [v2(), v4()] {
        let small = font.rasterize(WORD, 40.0, Palette::Light, BLACK).unwrap();
        let large = font.rasterize(WORD, 80.0, Palette::Light, BLACK).unwrap();
        assert_eq!(large.advance, small.advance * 2.0);
        assert!(
            (large.top - small.top * 2.0).abs() <= 2.0
                && (large.left - small.left * 2.0).abs() <= 2.0
        );
        assert!((large.width as f32 - small.width as f32 * 2.0).abs() <= 3.0);
        assert!((large.height as f32 - small.height as f32 * 2.0).abs() <= 3.0);
    }
}

#[test]
fn mono_font_uses_the_given_colour() {
    let font = v2();
    let colour = [200, 30, 60, 255];
    let image = font.rasterize(WORD, 64.0, Palette::Light, colour).unwrap();
    assert_eq!(opaque_colors(&image), HashSet::from([[200, 30, 60]]));
    for p in image.rgba.as_chunks::<4>().0.iter() {
        let a = u32::from(p[3]);
        for (channel, full) in p[..3].iter().zip([200u32, 30, 60]) {
            assert!(
                (i64::from(*channel) - i64::from((full * a + 127) / 255)).abs() <= 1,
                "{p:?}"
            );
        }
    }
    // The palette means nothing to a single-colour font.
    assert_eq!(
        image,
        font.rasterize(WORD, 64.0, Palette::Dark, colour).unwrap()
    );
    assert_eq!(
        image,
        font.rasterize(WORD, 64.0, Palette::Index(9), colour)
            .unwrap()
    );

    let faint = font
        .rasterize(WORD, 64.0, Palette::Light, [200, 30, 60, 128])
        .unwrap();
    assert_well_formed(&faint);
    assert_eq!(
        faint.rgba.as_chunks::<4>().0.iter().map(|p| p[3]).max(),
        Some(128)
    );
    assert_eq!((faint.width, faint.height), (image.width, image.height));
}

#[test]
fn tajweed_font_draws_in_colour() {
    let font = v4();
    // The marker: black line work, pale green centre, pink petals, teal bud.
    let marker = opaque_colors(&font.rasterize(MARKER, 64.0, Palette::Light, BLACK).unwrap());
    for colour in [[0, 0, 0], [0xd8, 0xe9, 0xd8], [0xff, 0x00, 0x80]] {
        assert!(
            marker.contains(&colour),
            "marker lacks {colour:?}: {marker:?}"
        );
    }
    // A word: black letters plus at least one tajweed colour.
    let word = opaque_colors(
        &font
            .rasterize(TAJWEED_WORD, 64.0, Palette::Light, BLACK)
            .unwrap(),
    );
    assert!(word.contains(&[0, 0, 0]));
    assert!(
        word.iter().any(|[r, g, b]| r != g || g != b),
        "no hue in {word:?}"
    );
    // `mono` plays no part in the QCF colour glyphs.
    assert_eq!(
        font.rasterize(TAJWEED_WORD, 64.0, Palette::Light, BLACK),
        font.rasterize(TAJWEED_WORD, 64.0, Palette::Light, [9, 200, 9, 77])
    );
}

#[test]
fn light_and_dark_palettes_differ() {
    let font = v4();
    for ch in [WORD, TAJWEED_WORD, MARKER] {
        let light = font.rasterize(ch, 64.0, Palette::Light, BLACK).unwrap();
        let dark = font.rasterize(ch, 64.0, Palette::Dark, BLACK).unwrap();
        assert_eq!(
            (light.width, light.height, light.left, light.top),
            (dark.width, dark.height, dark.left, dark.top)
        );
        assert_ne!(light.rgba, dark.rgba);
        let (light, dark) = (opaque_colors(&light), opaque_colors(&dark));
        assert!(light.contains(&[0, 0, 0]) && !light.contains(&[255, 255, 255]));
        assert!(dark.contains(&[255, 255, 255]) && !dark.contains(&[0, 0, 0]));
    }
}

#[test]
fn palettes_can_be_picked_by_index() {
    let font = v4();
    let draw = |palette| font.rasterize(TAJWEED_WORD, 48.0, palette, BLACK);
    assert_eq!(draw(Palette::Index(0)), draw(Palette::Light));
    assert_eq!(draw(Palette::Index(1)), draw(Palette::Dark));
    // Palette 3 is the light one without tajweed colours.
    assert_eq!(
        opaque_colors(&draw(Palette::Index(3)).unwrap()),
        HashSet::from([[0, 0, 0]])
    );
    assert!(draw(Palette::Index(6)).is_none());
}

#[test]
fn refuses_unusable_sizes() {
    for font in [v2(), v4()] {
        for size in [
            0.0,
            -12.0,
            f32::NAN,
            f32::INFINITY,
            f32::NEG_INFINITY,
            1024.5,
            1.0e9,
        ] {
            assert!(
                font.rasterize(WORD, size, Palette::Light, BLACK).is_none(),
                "{size}"
            );
        }
        for size in [f32::MIN_POSITIVE, 0.01, 1.0, 1024.0] {
            assert_well_formed(&font.rasterize(WORD, size, Palette::Light, BLACK).unwrap());
        }
    }
}

#[test]
fn rejects_garbage() {
    assert_eq!(PageFont::from_bytes(&[]).err(), Some(Error::InvalidFont));
    assert_eq!(PageFont::from_bytes(b"wOF2").err(), Some(Error::Woff2));
    assert_eq!(
        PageFont::from_bytes(&V2[..V2.len() / 2]).err(),
        Some(Error::Woff2)
    );
    assert_eq!(
        PageFont::from_bytes(b"<!doctype html><title>404</title>").err(),
        Some(Error::InvalidFont)
    );
    assert_eq!(
        PageFont::from_bytes(&[0u8; 4096]).err(),
        Some(Error::InvalidFont)
    );
}

/// Loads `bytes` and, if that works, exercises everything. Must not panic.
fn exercise(bytes: &[u8]) {
    let Ok(font) = PageFont::from_bytes(bytes) else {
        return;
    };
    let _ = (font.metrics_em(), font.is_color(), font.palette_count());
    let mut chars = font.chars();
    chars.truncate(400);
    chars.extend([WORD, MARKER, ' ']);
    for ch in chars {
        let _ = (font.has_glyph(ch), font.advance_em(ch));
        for palette in [Palette::Light, Palette::Dark] {
            if let Some(image) = font.rasterize(ch, 24.0, palette, BLACK) {
                assert_eq!(
                    image.rgba.len(),
                    image.width as usize * image.height as usize * 4
                );
            }
        }
    }
}

#[test]
fn survives_truncated_and_corrupted_fonts() {
    let mut seed = 0x9E37_79B9_7F4A_7C15u64;
    let mut random = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed as usize
    };
    for packed in [V2, V4] {
        let sfnt = wuff::decompress_woff2(packed).unwrap();
        for original in [packed, &sfnt[..]] {
            for cut in (0..original.len()).step_by(original.len() / 40 + 1) {
                exercise(&original[..cut]);
            }
            for _ in 0..120 {
                let mut bytes = original.to_vec();
                // Mostly hit the headers and table directory, where damage is not just a bad glyph.
                let reach = if random() % 3 == 0 {
                    bytes.len()
                } else {
                    bytes.len().min(2048)
                };
                for _ in 0..1 + random() % 6 {
                    let at = random() % reach;
                    bytes[at] = match random() % 3 {
                        0 => 0xFF,
                        1 => 0,
                        _ => random() as u8,
                    };
                }
                exercise(&bytes);
            }
        }
    }
}
