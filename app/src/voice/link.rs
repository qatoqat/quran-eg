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
use super::motion::Plan;
use super::presence::{self, Presence};

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
    /// Other places the listener weighed (look-alikes of this one).
    pub alts: Vec<Loc>,
    /// From a listener that plans (`ev`, `ev_end`, `seq`, `age`, `now`, `plan`, `hold`): where the reader is and how the
    /// next half second goes.
    pub plan: Option<Plan>,
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
    /// A new plan while the place shown stays (`{"replan":{..}}`), and the seconds of audio heard then.
    Replan(Plan, f32),
    /// The reader's voice came or went, or a silence began or ended, by the levels a listener sends when asked
    /// (`presence`), and how far the voice stands over the room's sound, dB.
    /// The voice came or went (`heard`); how far it stands over the room's sound (dB), and the audio time of the last
    /// loud level (where a silence began).
    Voice { heard: presence::Heard, range: Option<f32>, loud: Option<f32> },
    Audio(Audio),
    /// The connection or the listener failed; the link is over.
    Failed(String),
}

/// What the pointer's settings want from the listener beside its pointers (`{"want":{..}}`, which older listeners pass
/// over): plans, which come with the voice level, for tajweed timing; the voice level alone for the hold or the dim.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Want {
    pub plans: bool,
    pub voice: bool,
}

pub struct Link {
    /// The listener this link started (stopped with it), if it did not join a running one.
    child: Option<Child>,
    events: Receiver<Event>,
    visible: Sender<(Loc, Loc, bool)>,
    want: Sender<Want>,
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

/// What a message says: an event, or a frame's voice level (`{"voice":{"t":..,"db":..}}`, which the link thread turns
/// into the voice's presence).
enum Read {
    Event(Event),
    Voice(f32, f32),
}

/// A message from the listener, by its key: news of the microphone, a pointer (`loc`), a plan alone (`replan`) or a
/// voice level; `None` for anything else it sends (checker findings, tariq notes, later news). The microphone's:
/// - `{"mic":{"name":"…","rate":48000,"channels":2}}`, first to each client;
/// - `{"level":{"rms":-27.4,"peak":-11.9,"clip":false}}`, about 15 a second while samples arrive;
/// - `{"audio":"stalled"}` and `{"audio":"error","detail":"…"}`, once each time, until a level.
fn read(text: &str) -> Option<Read> {
    use serde_json::Value;
    let v: Value = serde_json::from_str(text).ok()?;
    if let Some(x) = v.get("voice") {
        return Some(Read::Voice(x.get("t")?.as_f64()? as f32, x.get("db")?.as_f64()? as f32));
    }
    if let Some(r) = v.get("replan") {
        return Some(Read::Event(Event::Replan(plan(r)?, r.get("t").and_then(Value::as_f64).unwrap_or(0.0) as f32)));
    }
    event(&v).map(Read::Event)
}

fn event(v: &serde_json::Value) -> Option<Event> {
    use serde_json::Value;
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
    heard(v).map(Event::Heard)
}

/// A plan's fields, in a pointer message or a plan alone.
fn plan(v: &serde_json::Value) -> Option<Plan> {
    let pair = |k: &str| v.get(k)?.as_array().and_then(|a| Some((a.first()?.as_f64()?, a.get(1)?.as_f64()?)));
    let num = |k: &str| v.get(k).and_then(|x| x.as_f64());
    let (word, letter) = pair("ev")?;
    let now = num("now")? as f32;
    let knots = v.get("plan")?.as_array()?.iter().filter_map(|k| Some((k.get(0)?.as_f64()? as f32, k.get(1)?.as_f64()? as f32))).collect();
    Some(Plan {
        seq: num("seq")? as u32,
        age: num("age")? as f32,
        ev: (word as u32, letter as u16),
        // A listener that does not say where the evidenced letter ends: no further than where the reader is.
        ev_end: num("ev_end").map_or(now, |e| e as f32),
        now,
        knots,
        hold: pair("hold").map(|(a, b)| (a as f32, b as f32)),
    })
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
        alts: v.get("alts").and_then(|a| a.as_array()).map_or_else(Vec::new, |a| a.iter().filter_map(|l| parse_loc(l.as_str()?)).collect()),
        plan: plan(v),
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
        let (want, want_rx) = channel();
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        std::thread::Builder::new()
            .name("voice-link".into())
            .spawn(move || run(&events_tx, (&visible_rx, &want_rx), &flag, &ctx))
            .map_err(|e| format!("could not start the listener's thread: {e}"))?;
        Ok(Link { child, events, visible, want, stop, log })
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

    /// What the settings want from the listener beside its pointers; it goes out once connected.
    pub fn send_want(&self, want: Want) {
        let _ = self.want.send(want);
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

fn run(events: &Sender<Event>, (visible, want): (&Receiver<(Loc, Loc, bool)>, &Receiver<Want>), stop: &AtomicBool, ctx: &egui::Context) {
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
    let (mut gate, mut presence) = (Gate::default(), Presence::default());
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
        if let Some(w) = want.try_iter().last() {
            let msg = format!(r#"{{"want":{{"plans":{},"voice":{}}}}}"#, w.plans, w.voice);
            if let Err(e) = ws.send(Message::text(msg)) {
                return send(Event::Failed(format!("listener connection: {e}")));
            }
        }
        match ws.read() {
            Ok(Message::Text(t)) => match read(t.as_str()) {
                // Levels that would not move the meter are not worth waking the app for, nor the voice's while its
                // presence stays.
                Some(Read::Event(Event::Audio(a))) if !gate.pass(&a) => {}
                Some(Read::Event(e)) => send(e),
                Some(Read::Voice(t, db)) => {
                    if let Some(heard) = presence.push(t, db) {
                        send(Event::Voice { heard, range: presence.range(), loud: presence.last_loud() });
                    }
                }
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

    /// The events a message gives (a voice level is the link thread's).
    fn parse(text: &str) -> Option<Event> {
        match read(text)? {
            Read::Event(e) => Some(e),
            Read::Voice(..) => None,
        }
    }

    #[test]
    fn pointer_messages_parse_and_others_do_not() {
        let Some(Event::Heard(h)) = parse(r#"{"loc":"57:16:4","word":51234,"letter":2,"letters":5,"state":"Locked","conf":0.97,"speed":6.1,"t":12.5,"lat":0.081,"alts":["57:17:1"],"slid":["2:5:1","31:5:1"]}"#) else {
            panic!("a pointer message")
        };
        assert_eq!((h.loc, h.word, h.letter, h.letters, h.state.as_str()), ((57, 16, 4), 51234, 2, 5, "Locked"));
        assert_eq!((h.slid, h.alts), (Some(((2, 5, 1), (31, 5, 1))), vec![(57, 17, 1)]));
        assert_eq!(h.plan, None, "no plan from an older listener");
        assert!(parse(r#"{"findings":[]}"#).is_none());
        assert!(parse(r#"{"loc":"57:16","word":1,"state":"Locked"}"#).is_none(), "a location has three parts");
        assert!(parse("not json").is_none());
    }

    #[test]
    fn plans_parse_with_their_pointer_or_alone_and_voice_levels_are_the_threads() {
        let msg = r#"{"loc":"36:16:4","word":37000,"letter":1,"letters":4,"state":"Locked","conf":0.97,"speed":6.1,"t":12.5,"lat":0.081,"alts":[],"ev":[37000,1],"ev_end":37000.475,"seq":7,"age":0.120,"now":37000.300,"plan":[[0.200,0.100],[0.600,0.175]],"hold":[0.100,0.900]}"#;
        let Some(Event::Heard(h)) = parse(msg) else { panic!("a pointer message") };
        let p = h.plan.expect("its plan");
        assert_eq!((p.seq, p.ev, p.knots.len(), p.hold), (7, (37000, 1), 2, Some((0.1, 0.9))));
        assert!((p.now - 37000.3).abs() < 1e-2 && (p.ev_end - 37000.475).abs() < 1e-2 && (p.position(0.4) - (p.now + 0.1375)).abs() < 5e-3);
        let Some(Event::Replan(q, t)) = parse(r#"{"replan":{"t":12.82,"ev":[37000,2],"seq":8,"age":0.050,"now":37000.500,"plan":[]}}"#) else {
            panic!("a plan alone")
        };
        assert_eq!((q.seq, t, q.ev_end), (8, 12.82, q.now), "no ev_end: no further than now");
        assert!(parse(r#"{"voice":{"t":12.84,"db":-31.5}}"#).is_none(), "a level is the link thread's");
        assert!(matches!(read(r#"{"voice":{"t":12.84,"db":-31.5}}"#), Some(Read::Voice(t, db)) if t == 12.84 && db == -31.5));
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
