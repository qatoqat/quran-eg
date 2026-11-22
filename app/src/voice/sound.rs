//! The replay's sound: a recording played through the default output with cpal, silent until the replay's listener
//! has joined (`Cue::go`), then from its first sample, the first callback saying when that sample is heard (the
//! output's latency on from now) so that the listener's feed starts with it. The recording is read and the stream
//! opened on a thread of the sound's own, which holds the stream until the sound is dropped.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// How far the sound is: shared by its thread, its callback, the replay's link and the app.
pub struct Cue {
    base: Instant,
    /// The output is open, playing silence.
    ready: AtomicBool,
    /// The recording plays from its start.
    go: AtomicBool,
    /// When its first sample is heard, in ns after `base` plus one (0: not known yet).
    heard: AtomicU64,
    /// The recording's rate and length in samples (0 until it is read), and how many of them have gone out.
    rate: AtomicU32,
    length: AtomicU64,
    played: AtomicU64,
    error: Mutex<Option<String>>,
}

impl Cue {
    fn new() -> Cue {
        Cue {
            base: Instant::now(),
            ready: AtomicBool::new(false),
            go: AtomicBool::new(false),
            heard: AtomicU64::new(0),
            rate: AtomicU32::new(0),
            length: AtomicU64::new(0),
            played: AtomicU64::new(0),
            error: Mutex::new(None),
        }
    }

    pub fn ready(&self) -> bool {
        self.ready.load(Ordering::Acquire)
    }

    /// Plays the recording from its start (the output's next buffer on).
    pub fn go(&self) {
        self.go.store(true, Ordering::Release);
    }

    pub fn playing(&self) -> bool {
        self.go.load(Ordering::Acquire)
    }

    /// When the recording's first sample is heard, once the output has said.
    pub fn heard(&self) -> Option<Instant> {
        let ns = self.heard.load(Ordering::Acquire);
        (ns > 0).then(|| self.base + std::time::Duration::from_nanos(ns - 1))
    }

    fn heard_at(&self, at: Instant) {
        let ns = u64::try_from(at.saturating_duration_since(self.base).as_nanos()).unwrap_or(u64::MAX - 1) + 1;
        let _ = self.heard.compare_exchange(0, ns, Ordering::AcqRel, Ordering::Acquire);
    }

    /// Seconds played and the recording's length, once it is read.
    pub fn progress(&self) -> Option<(f32, f32)> {
        let rate = self.rate.load(Ordering::Relaxed) as f32;
        let length = self.length.load(Ordering::Relaxed) as f32;
        (rate > 0.0).then(|| (self.played.load(Ordering::Relaxed) as f32 / rate, length / rate))
    }

    /// The whole recording has gone to the output.
    pub fn over(&self) -> bool {
        let length = self.length.load(Ordering::Relaxed);
        length > 0 && self.played.load(Ordering::Relaxed) >= length
    }

    pub fn error(&self) -> Option<String> {
        self.error.lock().ok().and_then(|e| e.clone())
    }

    fn fail(&self, e: String) {
        if let Ok(mut slot) = self.error.lock() {
            slot.get_or_insert(e);
        }
    }
}

/// A recording playing (or about to); dropped, it stops.
pub struct Sound {
    pub cue: Arc<Cue>,
    /// Dropped with the sound: its thread lets the stream go.
    _stop: Sender<()>,
}

impl Sound {
    /// Reads `wav` and opens the output on a thread, silent until the cue's `go`.
    pub fn start(wav: PathBuf) -> Result<Sound, String> {
        let cue = Arc::new(Cue::new());
        let (stop, stopped) = channel::<()>();
        let shared = cue.clone();
        std::thread::Builder::new()
            .name("replay-sound".into())
            .spawn(move || match open(&wav, &shared) {
                Ok(stream) => {
                    // Nothing is sent: this returns when the sound is dropped.
                    let _ = stopped.recv();
                    drop(stream);
                }
                Err(e) => shared.fail(e),
            })
            .map_err(|e| format!("could not start the sound's thread: {e}"))?;
        Ok(Sound { cue, _stop: stop })
    }
}

/// The recording's samples and rate, then the default output open and playing silence.
fn open(wav: &Path, cue: &Arc<Cue>) -> Result<cpal::Stream, String> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    let bytes = std::fs::read(wav).map_err(|e| format!("{}: {e}", wav.display()))?;
    let (samples, rate) = crate::recorder::read_wav(&bytes).ok_or_else(|| format!("{}: not a recording this app made", wav.display()))?;
    drop(bytes);
    cue.rate.store(rate, Ordering::Relaxed);
    cue.length.store(samples.len() as u64, Ordering::Relaxed);
    let dev = cpal::default_host().default_output_device().ok_or("no sound output found")?;
    let default = dev.default_output_config().map_err(|e| format!("sound output: {e}"))?;
    let channels = default.channels();
    // The recording's own rate where the output takes it, else the output's, the recording stepped through to it.
    let takes = |c: &cpal::SupportedStreamConfigRange| c.channels() == channels && (c.min_sample_rate()..=c.max_sample_rate()).contains(&rate);
    let out_rate = if dev.supported_output_configs().is_ok_and(|mut all| all.any(|c| takes(&c))) { rate } else { default.sample_rate() };
    let config = cpal::StreamConfig { channels, sample_rate: out_rate, buffer_size: cpal::BufferSize::Default };
    let step = f64::from(rate) / f64::from(out_rate);
    let errors = {
        let cue = cue.clone();
        move |e: cpal::Error| cue.fail(format!("sound output: {e}"))
    };
    let (ch, shared) = (usize::from(channels), cue.clone());
    let stream = match default.sample_format() {
        cpal::SampleFormat::I16 => dev.build_output_stream(config, feed::<i16>(samples, step, ch, shared), errors, None),
        _ => dev.build_output_stream(config, feed::<f32>(samples, step, ch, shared), errors, None),
    }
    .map_err(|e| format!("sound output: {e}"))?;
    stream.play().map_err(|e| format!("sound output: {e}"))?;
    cue.ready.store(true, Ordering::Release);
    Ok(stream)
}

/// The data callback: silence until the cue's `go`, then the recording, `step` of its samples a frame, on every channel.
fn feed<T: cpal::SizedSample + cpal::FromSample<f32>>(samples: Vec<i16>, step: f64, channels: usize, cue: Arc<Cue>) -> impl FnMut(&mut [T], &cpal::OutputCallbackInfo) + Send + 'static {
    let mut at = 0.0f64;
    move |out: &mut [T], info: &cpal::OutputCallbackInfo| {
        if !cue.playing() {
            out.fill(T::EQUILIBRIUM);
            return;
        }
        if at == 0.0 && !out.is_empty() {
            let t = info.timestamp();
            cue.heard_at(Instant::now() + t.playback.duration_since(t.callback));
        }
        for frame in out.chunks_mut(channels) {
            frame.fill(T::from_sample(sample(&samples, at)));
            at += step;
        }
        cue.played.store((at as u64).min(samples.len() as u64), Ordering::Relaxed);
    }
}

/// The recording at `at` (in samples), between its two nearest samples; silence past its end.
fn sample(samples: &[i16], at: f64) -> f32 {
    let i = at as usize;
    let Some(&a) = samples.get(i) else { return 0.0 };
    let b = samples.get(i + 1).copied().unwrap_or(a);
    let f = (at - i as f64) as f32;
    (f32::from(a) + (f32::from(b) - f32::from(a)) * f) / 32768.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samples_are_read_between_their_neighbours_and_end_in_silence() {
        let s = [0, 16384, -16384];
        assert_eq!((sample(&s, 0.0), sample(&s, 1.0), sample(&s, 0.5)), (0.0, 0.5, 0.25));
        assert_eq!((sample(&s, 2.0), sample(&s, 2.5), sample(&s, 3.0)), (-0.5, -0.5, 0.0));
    }

    #[test]
    fn the_cue_says_when_and_how_far() {
        let cue = Cue::new();
        assert_eq!((cue.heard(), cue.progress(), cue.over(), cue.playing()), (None, None, false, false));
        let at = cue.base + std::time::Duration::from_millis(40);
        cue.heard_at(at);
        cue.heard_at(at + std::time::Duration::from_secs(1));
        assert_eq!(cue.heard(), Some(at), "the first sample's time is said once");
        cue.rate.store(16000, Ordering::Relaxed);
        cue.length.store(32000, Ordering::Relaxed);
        cue.played.store(8000, Ordering::Relaxed);
        assert_eq!((cue.progress(), cue.over()), (Some((0.5, 2.0)), false));
        cue.played.store(32000, Ordering::Relaxed);
        assert!(cue.over());
        cue.fail("first".into());
        cue.fail("second".into());
        assert_eq!(cue.error().as_deref(), Some("first"));
    }
}
