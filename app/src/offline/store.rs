//! The kept copy of the API's answers: a file for each request in `<cache dir>/quran-eg/api`, named by the md5 of
//! its URL, the URL on its first line and the answer after it. A file is written whole or not at all (a temporary
//! file renamed over it), and only there: it is the reader's own copy. An answer asked for again replaces the kept
//! one only if it holds what that one held (`holds`): an update never takes a verse, a word or a list away.

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{Duration, SystemTime};

use md5::{Digest, Md5};
use quran_core::settings::storage::{APP_DIR, write_atomic};
use serde_json::Value;

pub struct Store {
    dir: PathBuf,
}

/// A kept answer, and when it was fetched.
pub struct Kept {
    pub body: Vec<u8>,
    pub fetched: SystemTime,
}

impl Kept {
    pub fn age(&self) -> Duration {
        self.fetched.elapsed().unwrap_or_default()
    }
}

/// The reader's own copy, opened once (none in tests, which must leave it alone).
pub fn user() -> Option<&'static Store> {
    static STORE: OnceLock<Option<Store>> = OnceLock::new();
    let open = || dirs::cache_dir().filter(|_| !cfg!(test)).map(|dir| Store::at(dir.join(APP_DIR).join("api")));
    STORE.get_or_init(open).as_ref()
}

impl Store {
    pub fn at(dir: PathBuf) -> Store {
        Store { dir }
    }

    fn file(&self, url: &str) -> PathBuf {
        self.dir.join(Md5::digest(url.as_bytes()).iter().map(|b| format!("{b:02x}")).collect::<String>())
    }

    /// The answer kept for `url`.
    pub fn read(&self, url: &str) -> Option<Kept> {
        let file = self.file(url);
        let mut bytes = std::fs::read(&file).ok()?;
        if !bytes.starts_with(url.as_bytes()) || bytes.get(url.len()) != Some(&b'\n') {
            return None;
        }
        bytes.drain(..=url.len());
        Some(Kept { body: bytes, fetched: std::fs::metadata(&file).and_then(|m| m.modified()).ok()? })
    }

    /// When the answer kept for `url` was fetched, if one is.
    pub fn fetched(&self, url: &str) -> Option<SystemTime> {
        let file = std::fs::File::open(self.file(url)).ok()?;
        let mut first = String::new();
        BufReader::new(&file).read_line(&mut first).ok()?;
        (first.strip_suffix('\n') == Some(url)).then(|| file.metadata().and_then(|m| m.modified()).ok()).flatten()
    }

    pub fn write(&self, url: &str, body: &[u8]) -> std::io::Result<()> {
        let mut bytes = Vec::with_capacity(url.len() + 1 + body.len());
        bytes.extend_from_slice(url.as_bytes());
        bytes.push(b'\n');
        bytes.extend_from_slice(body);
        write_atomic(&self.file(url), &bytes)
    }

    /// Every kept answer: its URL and the size of its file.
    pub fn list(&self) -> Vec<(String, u64)> {
        let Ok(entries) = std::fs::read_dir(&self.dir) else { return Vec::new() };
        let kept = |entry: std::fs::DirEntry| {
            // (A temporary file of a write under way has a dot in its name.)
            if entry.file_name().to_string_lossy().contains('.') {
                return None;
            }
            let file = std::fs::File::open(entry.path()).ok()?;
            let mut first = String::new();
            BufReader::new(&file).read_line(&mut first).ok()?;
            Some((first.strip_suffix('\n')?.to_string(), file.metadata().ok()?.len()))
        };
        entries.filter_map(|e| kept(e.ok()?)).collect()
    }
}

/// Whether a new answer holds what the kept one did: its fields, every verse it had (in order, with as many words),
/// and something in every list that had something. A kept answer that can't be read is replaced by any that can.
pub fn holds(kept: &[u8], fresh: &[u8]) -> bool {
    let Ok(Value::Object(fresh)) = serde_json::from_slice(fresh) else { return false };
    let Ok(Value::Object(kept)) = serde_json::from_slice(kept) else { return true };
    let words = |v: &Value| v.get("words").and_then(Value::as_array).map(Vec::len);
    let verses = |list: &[Value]| list.iter().map(|v| (v.get("verse_key").and_then(Value::as_str).map(str::to_owned), words(v))).collect::<Vec<_>>();
    kept.iter().all(|(key, old)| match (old, fresh.get(key)) {
        (_, None) => false,
        (Value::Array(old), Some(Value::Array(new))) if key == "verses" => verses(old) == verses(new),
        (Value::Array(old), Some(Value::Array(new))) => old.is_empty() || !new.is_empty(),
        (Value::Object(_), Some(new)) if key == "verse" => words(old) == words(new),
        (_, Some(_)) => true,
    })
}

#[cfg(test)]
pub mod tests {
    use serde_json::json;

    use super::*;

    /// A folder of its own in the temporary directory, empty.
    pub fn scratch_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("quran-eg-kept-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    /// A store of its own in the temporary directory, empty.
    pub fn scratch(name: &str) -> Store {
        Store::at(scratch_dir(name))
    }

    /// The answer kept for `url` as if fetched `days` ago.
    pub fn backdate(store: &Store, url: &str, days: u64) {
        let file = std::fs::File::options().write(true).open(store.file(url)).unwrap();
        file.set_modified(SystemTime::now() - Duration::from_secs(days * 24 * 60 * 60)).unwrap();
    }

    pub fn remove(store: &Store) {
        let _ = std::fs::remove_dir_all(&store.dir);
    }

    #[test]
    fn an_answer_is_kept_under_its_url_and_only_there() {
        assert!(user().is_none(), "tests never touch the reader's own copy");
        let store = scratch("store");
        let url = "https://api.quran.com/api/v4/verses/by_chapter/1?language=en&page=1";
        assert!(store.read(url).is_none() && store.fetched(url).is_none() && store.list().is_empty());
        store.write(url, b"{\"verses\":[]}\n").unwrap();
        let kept = store.read(url).unwrap();
        assert_eq!(kept.body, b"{\"verses\":[]}\n");
        assert!(kept.age() < Duration::from_secs(60) && store.fetched(url).is_some());
        // Another URL is another file; a newer answer replaces the old.
        let other = format!("{url}0");
        assert!(store.read(&other).is_none() && store.fetched(&other).is_none());
        store.write(&other, b"[1]").unwrap();
        store.write(url, b"{}").unwrap();
        assert_eq!(store.read(url).unwrap().body, b"{}");
        let mut list = store.list();
        list.sort();
        assert_eq!(list, [(url.to_string(), url.len() as u64 + 3), (other.clone(), other.len() as u64 + 4)]);
        // A file whose first line is not its URL holds nothing (written by something else).
        std::fs::write(store.file(url), b"https://elsewhere\n{}").unwrap();
        assert!(store.read(url).is_none() && store.fetched(url).is_none());
        remove(&store);
    }

    #[test]
    fn a_new_answer_must_hold_what_the_kept_one_did() {
        let verses = |keys: &[(&str, usize)]| {
            let list: Vec<_> = keys.iter().map(|(k, n)| json!({ "verse_key": k, "words": vec![json!({}); *n] })).collect();
            serde_json::to_vec(&json!({ "verses": list, "pagination": {} })).unwrap()
        };
        let kept = verses(&[("1:1", 5), ("1:2", 5)]);
        assert!(holds(&kept, &verses(&[("1:1", 5), ("1:2", 5)])));
        assert!(!holds(&kept, &verses(&[("1:1", 5)])), "a verse gone");
        assert!(!holds(&kept, &verses(&[("1:1", 5), ("1:2", 4)])), "a word gone");
        assert!(!holds(&kept, br#"{"verses":[]}"#), "the pagination gone");
        assert!(holds(br#"{"chapters":[1,2]}"#, br#"{"chapters":[2]}"#) && !holds(br#"{"chapters":[1]}"#, br#"{"chapters":[]}"#));
        assert!(holds(br#"{"verse":{"words":[1,2]}}"#, br#"{"verse":{"words":[3,4]}}"#));
        assert!(!holds(br#"{"verse":{"words":[1,2]}}"#, br#"{"verse":{"words":[3]}}"#));
        assert!(holds(b"rubbish", b"{}") && !holds(b"{}", b"<html>"));
    }
}
