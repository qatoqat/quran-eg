//! Draws every glyph of a QCF page font into a PNG, right to left in codepoint
//! (reading) order, wrapped into rows.
//!
//! `cargo run -p qcf-glyphs --example page -- <font file> <out.png> [--dark] [--size PX] [--palette N]`

use std::fs::File;
use std::io::BufWriter;
use std::process::ExitCode;

use qcf_glyphs::{GlyphImage, PageFont, Palette};

const ROW_EMS: f32 = 13.0;
const MARGIN_EMS: f32 = 0.5;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let mut paths = Vec::new();
    let (mut dark, mut size, mut index) = (false, 64.0f32, None);
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--dark" => dark = true,
            "--size" => size = number(args.next())?,
            "--palette" => index = Some(number(args.next())?),
            _ => paths.push(arg),
        }
    }
    let [font_path, out_path] = paths.as_slice() else {
        return Err("usage: page <font file> <out.png> [--dark] [--size PX] [--palette N]".into());
    };

    let bytes = std::fs::read(font_path).map_err(|e| format!("{font_path}: {e}"))?;
    let font = PageFont::from_bytes(&bytes).map_err(|e| format!("{font_path}: {e}"))?;
    let palette = match index {
        Some(index) => Palette::Index(index),
        None if dark => Palette::Dark,
        None => Palette::Light,
    };
    let (background, ink) = if dark {
        ([0x1f, 0x21, 0x25], [255; 4])
    } else {
        ([255; 3], [0, 0, 0, 255])
    };

    let started = std::time::Instant::now();
    let mut glyphs: Vec<GlyphImage> = Vec::new();
    let mut failed = Vec::new();
    for ch in font.chars() {
        match font.rasterize(ch, size, palette, ink) {
            Some(glyph) => glyphs.push(glyph),
            None => failed.push(format!("U+{:04X}", ch as u32)),
        }
    }
    let elapsed = started.elapsed();
    if !failed.is_empty() {
        eprintln!("not drawn: {}", failed.join(" "));
    }

    let margin = (MARGIN_EMS * size).ceil();
    let row_width = (ROW_EMS * size).ceil();
    let above = glyphs.iter().map(|g| g.top).fold(0.0, f32::max).ceil();
    let below = glyphs
        .iter()
        .map(|g| g.height as f32 - g.top)
        .fold(0.0, f32::max)
        .ceil();
    let row_height = above + below + (0.15 * size).ceil();

    // Pen positions: x from the right edge of the row, y of the baseline.
    let mut placed = Vec::new();
    let (mut pen, mut row) = (0.0f32, 0u32);
    for glyph in &glyphs {
        if pen > 0.0 && pen + glyph.advance > row_width {
            pen = 0.0;
            row += 1;
        }
        pen += glyph.advance;
        placed.push((
            margin + row_width - pen,
            margin + row as f32 * row_height + above,
        ));
    }

    let width = (row_width + 2.0 * margin) as usize;
    let height = ((row + 1) as f32 * row_height + 2.0 * margin) as usize;
    let mut rgb: Vec<u8> = background
        .iter()
        .copied()
        .cycle()
        .take(width * height * 3)
        .collect();
    for (glyph, (pen_x, baseline)) in glyphs.iter().zip(placed) {
        let x0 = (pen_x.round() + glyph.left) as i64;
        let y0 = (baseline.round() - glyph.top) as i64;
        for (i, src) in glyph.rgba.as_chunks::<4>().0.iter().enumerate() {
            let x = x0 + (i % glyph.width as usize) as i64;
            let y = y0 + (i / glyph.width as usize) as i64;
            if x < 0 || y < 0 || x >= width as i64 || y >= height as i64 {
                continue;
            }
            let dst = &mut rgb[(y as usize * width + x as usize) * 3..][..3];
            let keep = 255 - u32::from(src[3]);
            for c in 0..3 {
                dst[c] =
                    (u32::from(src[c]) + (u32::from(dst[c]) * keep + 127) / 255).min(255) as u8;
            }
        }
    }

    let file = File::create(out_path).map_err(|e| format!("{out_path}: {e}"))?;
    let mut encoder = png::Encoder::new(BufWriter::new(file), width as u32, height as u32);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder
        .write_header()
        .map_err(|e| format!("{out_path}: {e}"))?;
    writer
        .write_image_data(&rgb)
        .map_err(|e| format!("{out_path}: {e}"))?;
    println!(
        "{out_path}: {} glyphs in {elapsed:.1?}, {width}x{height}",
        glyphs.len()
    );
    Ok(())
}

fn number<T: std::str::FromStr>(arg: Option<String>) -> Result<T, String> {
    arg.as_deref()
        .and_then(|a| a.parse().ok())
        .ok_or_else(|| "expected a number".to_string())
}
