//! Recording the reader's recitation, to train the voice pointer on their own voice. The microphone
//! goes to a mono 16-bit WAV at the device's rate; beside it a JSON sidecar says what was on screen:
//! the word the recording started at (the first word of the top line) and the reading position each
//! time it changed. quran-voice's `qv enroll` reads both: the text is known from the start word on,
//! so the audio can be aligned to it without a model having to find the place first. Files go to
//! `~/.local/share/quran-eg/recordings/`.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use quran_core::view::{Position, WordRef};

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

/// The newest recording's WAV (the one written last), if there is one.
pub fn newest() -> Option<PathBuf> {
    newest_in(&dir()?)
}

fn newest_in(dir: &Path) -> Option<PathBuf> {
    let wavs = std::fs::read_dir(dir).ok()?.flatten().filter(|e| e.path().extension().is_some_and(|x| x == "wav"));
    wavs.filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path()))).max_by_key(|(at, _)| *at).map(|(_, path)| path)
}

/// Where a recording started, as its sidecar says: the first word of the top line, and its page (0 if not said).
pub fn started_at(wav: &Path) -> Option<(WordRef, u16)> {
    let json: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(wav.with_extension("json")).ok()?).ok()?;
    let at = WordRef::parse(json.get("start")?.as_str()?)?;
    let page = json.get("page").and_then(serde_json::Value::as_u64).and_then(|p| u16::try_from(p).ok()).unwrap_or(0);
    Some((at, page))
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

/// A 16-bit PCM WAV, as `wav_bytes` writes it (more channels than one are mixed): its samples and rate.
pub fn read_wav(bytes: &[u8]) -> Option<(Vec<i16>, u32)> {
    if bytes.get(0..4)? != b"RIFF" || bytes.get(8..12)? != b"WAVE" {
        return None;
    }
    let (mut at, mut format) = (12, None);
    while let Some(head) = bytes.get(at..at + 8) {
        let size = u32::from_le_bytes(head[4..8].try_into().ok()?) as usize;
        // A data chunk cut short (a recording stopped while it was written) is read as far as it goes.
        let body = &bytes[at + 8..(at + 8).saturating_add(size).min(bytes.len())];
        match &head[..4] {
            b"fmt " if body.len() >= 16 => {
                let word = |i: usize| u16::from_le_bytes([body[i], body[i + 1]]);
                let rate = u32::from_le_bytes(body[4..8].try_into().ok()?);
                if word(0) != 1 || word(14) != 16 || word(2) == 0 || rate == 0 {
                    return None;
                }
                format = Some((usize::from(word(2)), rate));
            }
            b"data" => {
                let (channels, rate) = format?;
                let mix = |frame: &[u8]| {
                    let sum: i32 = frame.as_chunks::<2>().0.iter().map(|s| i32::from(i16::from_le_bytes(*s))).sum();
                    (sum / channels as i32) as i16
                };
                return Some((body.chunks_exact(2 * channels).map(mix).collect(), rate));
            }
            _ => {}
        }
        at = at.checked_add(8 + size + (size & 1))?;
    }
    None
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
    fn a_recording_reads_back_as_it_was_written() {
        let samples = [0.0, 0.5, -0.5, 1.0];
        let (read, rate) = read_wav(&wav_bytes(&samples, 48000)).unwrap();
        assert_eq!((read, rate), (vec![0, 16383, -16383, 32767], 48000));
        // Two channels are mixed; a chunk before the data is passed over; a cut data chunk reads as far as it goes.
        let mut stereo = wav_bytes(&[0.0; 4], 44100);
        stereo[22] = 2;
        stereo.splice(36..36, *b"LIST\x02\x00\x00\x00ab");
        let n = stereo.len();
        stereo[n - 8..].copy_from_slice(&[0x00, 0x40, 0x00, 0x20, 0x00, 0x80, 0x00, 0xc0]);
        assert_eq!(read_wav(&stereo), Some((vec![12288, -24576], 44100)));
        assert_eq!(read_wav(&stereo[..n - 3]), Some((vec![12288], 44100)));
        assert_eq!(read_wav(b"RIFF\0\0\0\0WAVEdata"), None, "no format");
        assert_eq!(read_wav(b"not a wav at all"), None);
    }

    #[test]
    fn the_newest_recording_and_where_it_started() {
        let dir = std::env::temp_dir().join(format!("quran-eg-recordings-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(newest_in(&dir), None);
        let (old, new) = (dir.join("recitation-1.wav"), dir.join("recitation-2.wav"));
        std::fs::write(&old, wav_bytes(&[0.0], 16000)).unwrap();
        std::fs::write(&new, wav_bytes(&[0.0], 16000)).unwrap();
        std::fs::write(dir.join("recitation-3.json"), "{}").unwrap();
        let past = std::time::SystemTime::now() - std::time::Duration::from_secs(60);
        std::fs::File::options().write(true).open(&old).unwrap().set_modified(past).unwrap();
        assert_eq!(newest_in(&dir), Some(new.clone()), "the WAV written last, a sidecar alone is no recording");
        std::fs::write(new.with_extension("json"), sidecar_json("18:46:1", 299, 48000, 3.0, &[])).unwrap();
        assert_eq!(started_at(&new), Some((WordRef { chapter: 18, verse: 46, word: 1 }, 299)));
        assert_eq!(started_at(&old), None, "no sidecar");
        std::fs::remove_dir_all(&dir).unwrap();
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
