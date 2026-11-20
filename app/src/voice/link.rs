//! The listener: quran-voice's `qv listen` (one already running, or the installed one started from its
//! folder) and the WebSocket on which it reports where the reader is and how its microphone is. A
//! thread reads its messages and sends it the words on screen, which it uses as a prior when looking
//! for the place.

use std::io::ErrorKind;
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use tungstenite::Message;
use tungstenite::stream::MaybeTlsStream;

use super::meter::Gate;

/// `qv listen`'s port, or the one `QURAN_EG_VOICE_PORT` names (tests run a stand-in there, clear of
/// a listener the user runs).
static PORT: LazyLock<u16> = LazyLock::new(|| std::env::var("QURAN_EG_VOICE_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(8765));
/// How long a starting listener may take to load its model and open the port.
const START_TIMEOUT: Duration = Duration::from_secs(30);
/// Earlier sessions' logs kept beside `listen.log`.
const LOGS_KEPT: usize = 5;

/// A word: (surah, ayah, word); surah 0 is the isti'adha.
pub type Loc = (u16, u16, u16);

/// One pointer message: where the listener hears the reader.
#[derive(Debug, Clone, PartialEq)]
pub struct Heard {
    pub loc: Loc,
    /// Index of the word in the whole Quran.
    pub word: u32,
    /// Base letter being read, of `letters` in the word.
    pub letter: u16,
    pub letters: u16,
    /// "Locked", "Tentative", "Drifting", "Paused" or "Searching".
    pub state: String,
    pub conf: f32,
    /// Reading speed, units per second.
    pub speed: f32,
    /// Seconds of audio heard when this was decided.
    pub t: f32,
    /// Slid into a look-alike ayah: the last shared word and where the reader went.
    pub slid: Option<(Loc, Loc)>,
}

/// The microphone the listener captures from (its first message to each client).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Device {
    pub name: String,
    /// Hz.
    pub rate: u32,
    pub channels: u16,
}

/// The raw input since the listener's last level message (about 15 a second): RMS and peak of its
/// mono mix in dBFS, and whether any sample reached full scale.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Level {
    pub rms: f32,
    pub peak: f32,
    pub clip: bool,
}

/// What the listener says about its microphone.
#[derive(Debug, Clone, PartialEq)]
pub enum Audio {
    Mic(Device),
    Level(Level),
    /// No samples have come from the device for over a second; the next level ends it.
    Stalled,
    /// The capture stream failed (the error); the next level ends it.
    Error(String),
}

#[derive(Debug, PartialEq)]
pub enum Event {
    Connected,
    Heard(Heard),
    Audio(Audio),
    /// The connection or the listener failed; the link is over.
    Failed(String),
}

pub struct Link {
    /// The listener this link started (stopped with it), if it did not join a running one.
    child: Option<Child>,
    events: Receiver<Event>,
    visible: Sender<(Loc, Loc, bool)>,
    stop: Arc<AtomicBool>,
    log: Option<PathBuf>,
}

/// The installed listener's folder: the setting, or `<data dir>/quran-voice`.
pub fn folder(setting: &str) -> Option<PathBuf> {
    let setting = setting.trim();
    if setting.is_empty() { dirs::data_dir().map(|d| d.join("quran-voice")) } else { Some(PathBuf::from(setting)) }
}

fn parse_loc(s: &str) -> Option<Loc> {
    let mut it = s.split(':').map(|x| x.parse::<u16>().ok());
    let loc = (it.next()??, it.next()??, it.next()??);
    it.next().is_none().then_some(loc)
}

/// A message from the listener, by its key: news of the microphone or a pointer (`loc`); `None` for
/// anything else it sends (checker findings, tariq notes, later news). The microphone's:
/// - `{"mic":{"name":"…","rate":48000,"channels":2}}`, first to each client;
/// - `{"level":{"rms":-27.4,"peak":-11.9,"clip":false}}`, about 15 a second while samples arrive;
/// - `{"audio":"stalled"}` and `{"audio":"error","detail":"…"}`, once each time, until a level.
pub fn parse(text: &str) -> Option<Event> {
    use serde_json::Value;
    let v: Value = serde_json::from_str(text).ok()?;
    if let Some(l) = v.get("level") {
        let rms = l.get("rms")?.as_f64()? as f32;
        let peak = l.get("peak").and_then(Value::as_f64).map_or(rms, |p| p as f32);
        let clip = l.get("clip").and_then(Value::as_bool).unwrap_or(false);
        return Some(Event::Audio(Audio::Level(Level { rms, peak, clip })));
    }
    if let Some(m) = v.get("mic") {
        let int = |k: &str| m.get(k).and_then(Value::as_u64).unwrap_or(0);
        let name = m.get("name").and_then(Value::as_str).unwrap_or_default().to_string();
        let (rate, channels) = (u32::try_from(int("rate")).unwrap_or(0), u16::try_from(int("channels")).unwrap_or(0));
        return Some(Event::Audio(Audio::Mic(Device { name, rate, channels })));
    }
    if let Some(a) = v.get("audio") {
        return match a.as_str()? {
            "stalled" => Some(Event::Audio(Audio::Stalled)),
            "error" => Some(Event::Audio(Audio::Error(v.get("detail").and_then(Value::as_str).unwrap_or_default().to_string()))),
            _ => None,
        };
    }
    heard(&v).map(Event::Heard)
}

/// A pointer message.
fn heard(v: &serde_json::Value) -> Option<Heard> {
    let num = |k: &str| v.get(k).and_then(|x| x.as_f64());
    let slid = v.get("slid").and_then(|s| s.as_array()).and_then(|a| Some((parse_loc(a.first()?.as_str()?)?, parse_loc(a.get(1)?.as_str()?)?)));
    Some(Heard {
        loc: parse_loc(v.get("loc")?.as_str()?)?,
        word: num("word")? as u32,
        letter: num("letter").unwrap_or(0.0) as u16,
        letters: num("letters").unwrap_or(0.0) as u16,
        state: v.get("state")?.as_str()?.to_string(),
        conf: num("conf").unwrap_or(0.0) as f32,
        speed: num("speed").unwrap_or(0.0) as f32,
        t: num("t").unwrap_or(0.0) as f32,
        slid,
    })
}

impl Link {
    /// Joins a listener already on the port, or starts the one installed in `dir` and joins it.
    pub fn start(dir: Option<PathBuf>, ctx: egui::Context) -> Result<Link, String> {
        let addr = SocketAddr::from(([127, 0, 0, 1], *PORT));
        let running = TcpStream::connect_timeout(&addr, Duration::from_millis(300)).is_ok();
        let (child, log) = if running {
            (None, None)
        } else {
            let dir = dir.ok_or("no data directory for the listener")?;
            let (child, log) = spawn(&dir)?;
            (Some(child), Some(log))
        };
        let (events_tx, events) = channel();
        let (visible, visible_rx) = channel();
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        std::thread::Builder::new()
            .name("voice-link".into())
            .spawn(move || run(&events_tx, &visible_rx, &flag, &ctx))
            .map_err(|e| format!("could not start the listener's thread: {e}"))?;
        Ok(Link { child, events, visible, stop, log })
    }

    /// What arrived since the last call.
    pub fn poll(&mut self) -> Vec<Event> {
        let mut out: Vec<Event> = self.events.try_iter().collect();
        // A listener this link started that has exited says why in its log.
        if let Some(child) = &mut self.child
            && let Ok(Some(status)) = child.try_wait()
        {
            let tail = self.log.as_ref().and_then(|l| std::fs::read_to_string(l).ok()).and_then(|t| t.lines().rev().find(|l| !l.trim().is_empty()).map(str::to_string));
            out.push(Event::Failed(format!("the listener stopped ({status}){}", tail.map_or(String::new(), |t| format!(": {t}")))));
            self.child = None;
        }
        out
    }

    /// The words on screen (first and last), so the listener looks there first; `by_reader`: the reader
    /// put them there (not the pointer's own scrolling or navigation), so a search may start there.
    pub fn send_visible(&self, first: Loc, last: Loc, by_reader: bool) {
        let _ = self.visible.send((first, last, by_reader));
    }

    /// Whether this link started the listener (and stops it).
    pub fn started_it(&self) -> bool {
        self.child.is_some()
    }
}

impl Drop for Link {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Starts `qv listen` from its installed folder; its output goes to `listen.log` there.
fn spawn(dir: &Path) -> Result<(Child, PathBuf), String> {
    let qv = dir.join("qv");
    if !qv.is_file() || !dir.join("model").join("model.bpk").is_file() || !dir.join("ref.json").is_file() {
        return Err(format!("no listener installed in {} (run scripts/install-listener.sh in quran-voice)", dir.display()));
    }
    let log = dir.join("listen.log");
    // The last few sessions' logs stay (listen.1.log is the one before), to look back at a session.
    for i in (1..LOGS_KEPT).rev() {
        let _ = std::fs::rename(dir.join(format!("listen.{i}.log")), dir.join(format!("listen.{}.log", i + 1)));
    }
    let _ = std::fs::rename(&log, dir.join("listen.1.log"));
    let out = std::fs::File::create(&log).map_err(|e| format!("{}: {e}", log.display()))?;
    let err = out.try_clone().map_err(|e| format!("{}: {e}", log.display()))?;
    let child = Command::new(&qv)
        .args(["listen", "--run", "model", "--reference", "ref.json", "--port", &(*PORT).to_string()])
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(out)
        .stderr(err)
        .spawn()
        .map_err(|e| format!("could not start {}: {e}", qv.display()))?;
    Ok((child, log))
}

fn run(events: &Sender<Event>, visible: &Receiver<(Loc, Loc, bool)>, stop: &AtomicBool, ctx: &egui::Context) {
    let send = |e: Event| {
        let _ = events.send(e);
        ctx.request_repaint();
    };
    let started = Instant::now();
    let mut ws = loop {
        if stop.load(Ordering::Relaxed) {
            return;
        }
        match tungstenite::connect(format!("ws://127.0.0.1:{}", *PORT)) {
            Ok((ws, _)) => break ws,
            Err(_) if started.elapsed() < START_TIMEOUT => std::thread::sleep(Duration::from_millis(250)),
            Err(e) => return send(Event::Failed(format!("could not reach the listener: {e}"))),
        }
    };
    // Short reads, so the words on screen go out promptly too.
    if let MaybeTlsStream::Plain(s) = ws.get_mut()
        && let Err(e) = s.set_read_timeout(Some(Duration::from_millis(50)))
    {
        return send(Event::Failed(format!("listener connection: {e}")));
    }
    send(Event::Connected);
    let mut gate = Gate::default();
    loop {
        if stop.load(Ordering::Relaxed) {
            let _ = ws.close(None);
            return;
        }
        if let Some((a, b, by_reader)) = visible.try_iter().last() {
            // Older listeners ignore "by".
            let by = if by_reader { r#","by":"reader""# } else { "" };
            let msg = format!(r#"{{"visible":["{}:{}:{}","{}:{}:{}"]{by}}}"#, a.0, a.1, a.2, b.0, b.1, b.2);
            if let Err(e) = ws.send(Message::text(msg)) {
                return send(Event::Failed(format!("listener connection: {e}")));
            }
        }
        match ws.read() {
            Ok(Message::Text(t)) => match parse(t.as_str()) {
                // Levels that would not move the meter are not worth waking the app for.
                Some(Event::Audio(a)) if !gate.pass(&a) => {}
                Some(e) => send(e),
                None => {}
            },
            Ok(Message::Close(_)) => return send(Event::Failed("the listener closed the connection".into())),
            Ok(_) => {}
            Err(tungstenite::Error::Io(e)) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
            // Mostly the listener exiting (it does not close the connection first).
            Err(_) => return send(Event::Failed("the listener went away".into())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pointer_messages_parse_and_others_do_not() {
        let Some(Event::Heard(h)) = parse(r#"{"loc":"57:16:4","word":51234,"letter":2,"letters":5,"state":"Locked","conf":0.97,"speed":6.1,"t":12.5,"lat":0.081,"alts":["57:17:1"],"slid":["2:5:1","31:5:1"]}"#) else {
            panic!("a pointer message")
        };
        assert_eq!((h.loc, h.word, h.letter, h.letters, h.state.as_str()), ((57, 16, 4), 51234, 2, 5, "Locked"));
        assert_eq!(h.slid, Some(((2, 5, 1), (31, 5, 1))));
        assert!(parse(r#"{"findings":[]}"#).is_none());
        assert!(parse(r#"{"loc":"57:16","word":1,"state":"Locked"}"#).is_none(), "a location has three parts");
        assert!(parse("not json").is_none());
    }

    #[test]
    fn microphone_messages_parse() {
        let level = |rms, peak, clip| Some(Event::Audio(Audio::Level(Level { rms, peak, clip })));
        assert_eq!(parse(r#"{"level":{"rms":-27.4,"peak":-11.9,"clip":true}}"#), level(-27.4, -11.9, true));
        assert_eq!(parse(r#"{"level":{"rms":-27.4}}"#), level(-27.4, -27.4, false), "the peak defaults to the RMS");
        assert_eq!(parse(r#"{"level":{"rms":null}}"#), None);
        assert_eq!(parse(r#"{"level":{}}"#), None);
        let usb = Device { name: r#"USB "Mic""#.into(), rate: 44100, channels: 1 };
        assert_eq!(parse(r#"{"mic":{"name":"USB \"Mic\"","rate":44100,"channels":1}}"#), Some(Event::Audio(Audio::Mic(usb))));
        assert_eq!(parse(r#"{"mic":{"rate":48000}}"#), Some(Event::Audio(Audio::Mic(Device { rate: 48000, ..Device::default() }))));
        assert_eq!(parse(r#"{"audio":"stalled"}"#), Some(Event::Audio(Audio::Stalled)));
        assert_eq!(parse(r#"{"audio":"error","detail":"unplugged"}"#), Some(Event::Audio(Audio::Error("unplugged".into()))));
        assert_eq!(parse(r#"{"audio":"weird"}"#), None, "unknown news is ignored");
    }
}
