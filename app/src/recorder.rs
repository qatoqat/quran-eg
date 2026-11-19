//! Recording the reader's recitation, to train the voice pointer on their own voice. The microphone
//! goes to a mono 16-bit WAV at the device's rate; beside it a JSON sidecar says what was on screen:
//! the word the recording started at (the first word of the top line) and the reading position each
//! time it changed. quran-voice's `qv enroll` reads both: the text is known from the start word on,
//! so the audio can be aligned to it without a model having to find the place first. Files go to
//! `~/.local/share/quran-eg/recordings/`.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use quran_core::view::Position;

pub struct Recorder {
    _stream: cpal::Stream,
    samples: Arc<Mutex<Vec<f32>>>,
    error: Arc<Mutex<Option<String>>>,
    rate: u32,
    started: Instant,
    stamp: String,
    start: String,
    page: u16,
    /// Reading position changes: seconds since the start, and the position.
    positions: Vec<(f32, String)>,
}

/// `s:a:w`, always with the word (WordRef's Display drops `:1`).
fn loc(p: &Position) -> String {
    format!("{}:{}:{}", p.at.chapter, p.at.verse, p.at.word)
}

pub fn dir() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join("quran-eg").join("recordings"))
}

impl Recorder {
    /// Starts recording from the default microphone at reading position `at`.
    pub fn start(at: &Position) -> Result<Recorder, String> {
        use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
        let dev = cpal::default_host().default_input_device().ok_or("no microphone found")?;
        let cfg = dev.default_input_config().map_err(|e| format!("microphone: {e}"))?;
        let (rate, channels) = (cfg.sample_rate(), cfg.channels() as usize);
        let samples = Arc::new(Mutex::new(Vec::with_capacity(rate as usize * 60)));
        let error = Arc::new(Mutex::new(None));
        let (s, err) = (samples.clone(), error.clone());
        let stream = dev
            .build_input_stream(
                cfg.into(),
                move |data: &[f32], _| {
                    if let Ok(mut s) = s.lock() {
                        s.extend(data.chunks(channels).map(|f| f.iter().sum::<f32>() / channels as f32));
                    }
                },
                move |e| {
                    if let Ok(mut slot) = err.lock() {
                        *slot = Some(e.to_string());
                    }
                },
                None,
            )
            .map_err(|e| format!("microphone: {e}"))?;
        stream.play().map_err(|e| format!("microphone: {e}"))?;
        let start = loc(at);
        Ok(Recorder {
            _stream: stream,
            samples,
            error,
            rate,
            started: Instant::now(),
            stamp: stamp(),
            positions: vec![(0.0, start.clone())],
            start,
            page: at.page,
        })
    }

    pub fn elapsed(&self) -> f32 {
        self.started.elapsed().as_secs_f32()
    }

    pub fn start_word(&self) -> &str {
        &self.start
    }

    /// A stream error, if the microphone failed while recording.
    pub fn error(&self) -> Option<String> {
        self.error.lock().ok().and_then(|e| e.clone())
    }

    /// Notes the reading position when it changes (scrolling while reading).
    pub fn track(&mut self, at: Option<&Position>) {
        let Some(at) = at else { return };
        let here = loc(at);
        if self.positions.last().is_none_or(|p| p.1 != here) {
            self.positions.push((self.elapsed(), here));
        }
    }

    /// Stops and writes `<stamp>.wav` and `<stamp>.json`; returns the WAV's path.
    pub fn finish(self) -> Result<PathBuf, String> {
        let secs = self.elapsed();
        let Recorder { _stream, samples, rate, stamp, start, page, positions, .. } = self;
        drop(_stream);
        let samples = samples.lock().map_err(|_| "recording buffer poisoned")?.clone();
        let dir = dir().ok_or("no data directory")?;
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let wav = dir.join(format!("{stamp}.wav"));
        std::fs::write(&wav, wav_bytes(&samples, rate)).map_err(|e| format!("{}: {e}", wav.display()))?;
        let sidecar = sidecar_json(&start, page, rate, secs, &positions);
        let json = wav.with_extension("json");
        std::fs::write(&json, sidecar).map_err(|e| format!("{}: {e}", json.display()))?;
        Ok(wav)
    }
}

fn stamp() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    format!("recitation-{secs}")
}

/// Mono 16-bit PCM WAV.
pub fn wav_bytes(samples: &[f32], rate: u32) -> Vec<u8> {
    let n = samples.len() as u32;
    let mut out = Vec::with_capacity(44 + 2 * samples.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + 2 * n).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&[1, 0, 1, 0]);
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes());
    out.extend_from_slice(&[2, 0, 16, 0]);
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(2 * n).to_le_bytes());
    for &v in samples {
        out.extend_from_slice(&((v.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    out
}

/// Seconds rounded to hundredths (in f64, so the JSON reads 12.35, not 12.350000381).
fn cents(t: f32) -> f64 {
    (t as f64 * 100.0).round() / 100.0
}

pub fn sidecar_json(start: &str, page: u16, rate: u32, secs: f32, positions: &[(f32, String)]) -> String {
    serde_json::json!({
        "app": "quran-eg",
        "mushaf": "madani-15",
        "start": start,
        "page": page,
        "rate": rate,
        "secs": cents(secs),
        "positions": positions.iter().map(|(t, p)| serde_json::json!([cents(*t), p])).collect::<Vec<_>>(),
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_header_and_length() {
        let w = wav_bytes(&[0.0, 0.5, -1.0, 2.0], 48000);
        assert_eq!(&w[0..4], b"RIFF");
        assert_eq!(&w[8..16], b"WAVEfmt ");
        assert_eq!(u32::from_le_bytes(w[24..28].try_into().unwrap()), 48000);
        assert_eq!(u32::from_le_bytes(w[40..44].try_into().unwrap()), 8);
        assert_eq!(w.len(), 44 + 8);
        assert_eq!(i16::from_le_bytes(w[50..52].try_into().unwrap()), 32767, "clamped");
    }

    #[test]
    fn sidecar_names_the_start_word_and_positions() {
        let j: serde_json::Value = serde_json::from_str(&sidecar_json("2:238:1", 39, 48000, 12.345, &[(0.0, "2:238:1".into()), (6.5, "2:239:1".into())])).unwrap();
        assert_eq!(j["start"], "2:238:1");
        assert_eq!(j["page"], 39);
        assert_eq!(j["secs"], 12.35);
        assert_eq!(j["positions"][1][1], "2:239:1");
    }
}
