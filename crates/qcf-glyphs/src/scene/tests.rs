use skrifa::color::ColorStop;
use skrifa::raw::types::Point as FontPoint;
use skrifa::{FontRef, MetadataProvider};

use super::*;
use crate::raster::render;

const RED: Color = Color {
    blue: 0,
    green: 0,
    red: 255,
    alpha: 255,
};

fn with_recorder<T>(run: impl FnOnce(&mut Recorder<'_, '_>, GlyphId, GlyphId) -> T) -> T {
    let sfnt = wuff::decompress_woff2(include_bytes!("../../tests/fixtures/v2_p1.woff2")).unwrap();
    let font = FontRef::new(&sfnt).unwrap();
    let outlines = font.outline_glyphs();
    let glyph = |ch: char| font.charmap().map(ch).unwrap();
    let mut recorder = Recorder {
        outlines: &outlines,
        colors: &[RED],
        foreground: [0, 0, 255, 255],
        transforms: Vec::new(),
        scene: Scene::default(),
        failed: false,
    };
    run(&mut recorder, glyph('\u{FC41}'), glyph('\u{FC45}'))
}

fn solid(palette_index: u16) -> Brush<'static> {
    Brush::Solid {
        palette_index,
        alpha: 1.0,
    }
}

#[test]
fn clip_then_fill_equals_fill_glyph() {
    let (clipped, direct) = with_recorder(|r, word, _| {
        r.push_clip_glyph(word);
        r.fill(solid(0));
        r.pop_clip();
        let clipped = std::mem::take(&mut r.scene);
        r.fill_glyph(word, None, solid(0));
        assert!(!r.failed);
        (
            render(&clipped, 0.02).unwrap(),
            render(&r.scene, 0.02).unwrap(),
        )
    });
    assert!(clipped.width > 20 && clipped.rgba.as_chunks::<4>().0.contains(&[255, 0, 0, 255]));
    assert_eq!(
        (clipped.width, clipped.height, clipped.left, clipped.top),
        (direct.width, direct.height, direct.left, direct.top)
    );
    assert_eq!(clipped.rgba, direct.rgba);
}

#[test]
fn nested_clips_intersect() {
    let (word_only, both) = with_recorder(|r, word, marker| {
        r.fill_glyph(word, None, solid(0));
        let word_only = render(&std::mem::take(&mut r.scene), 0.02).unwrap();
        // The marker glyph, moved onto the word, as a second clip.
        r.push_clip_glyph(word);
        r.push_transform(Transform {
            dx: 1500.0,
            ..Transform::IDENTITY
        });
        r.push_clip_glyph(marker);
        r.pop_transform();
        r.fill(solid(FOREGROUND));
        r.pop_clip();
        r.pop_clip();
        assert!(!r.failed);
        (word_only, render(&r.scene, 0.02).unwrap())
    });
    let ink = |image: &crate::raster::Image| {
        image
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| p[3] > 0)
            .count()
    };
    assert!(ink(&both) > 0 && ink(&both) < ink(&word_only) / 2);
    assert!(
        both.rgba
            .as_chunks::<4>()
            .0
            .iter()
            .all(|p| p[0] == 0 && p[1] == 0 && p[2] == p[3])
    );
}

#[test]
fn transforms_move_shapes() {
    with_recorder(|r, word, _| {
        r.fill_glyph(word, None, solid(0));
        r.push_transform(Transform {
            dx: 1000.0,
            dy: -500.0,
            ..Transform::IDENTITY
        });
        r.push_transform(Transform {
            xx: 2.0,
            yy: 2.0,
            ..Transform::IDENTITY
        });
        r.fill_glyph(word, None, solid(0));
        let [Op::FillShape(plain, _), Op::FillShape(moved, _)] = &r.scene.ops[..] else {
            panic!()
        };
        assert_eq!(moved.bounds.x0, plain.bounds.x0 * 2.0 + 1000.0);
        assert_eq!(moved.bounds.y1, plain.bounds.y1 * 2.0 - 500.0);
        r.pop_transform();
        r.pop_transform();
        assert!(r.transforms.is_empty());
    });
}

#[test]
fn unsupported_paints_fail_the_glyph() {
    let stops = [ColorStop {
        offset: 0.0,
        palette_index: 0,
        alpha: 1.0,
    }];
    let gradient = || Brush::LinearGradient {
        p0: FontPoint::new(0.0, 0.0),
        p1: FontPoint::new(100.0, 0.0),
        color_stops: &stops,
        extend: skrifa::color::Extend::Pad,
    };
    let failed = |run: &dyn Fn(&mut Recorder<'_, '_>, GlyphId)| {
        with_recorder(|r, word, _| {
            run(r, word);
            r.failed
        })
    };
    assert!(!failed(&|r, word| r.fill_glyph(word, None, solid(0))));
    assert!(!failed(&|r, _| r.push_layer(CompositeMode::SrcOver)));
    assert!(failed(&|r, word| r.fill_glyph(word, None, gradient())));
    assert!(failed(&|r, _| r.fill(gradient())));
    assert!(failed(&|r, _| r.push_layer(CompositeMode::Multiply)));
    assert!(
        failed(&|r, word| r.fill_glyph(word, None, solid(7))),
        "colour outside the palette"
    );
    assert!(
        failed(&|r, _| r.fill_glyph(GlyphId::new(60000), None, solid(0))),
        "missing layer glyph"
    );
}
