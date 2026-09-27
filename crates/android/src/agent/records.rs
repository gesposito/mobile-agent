//! What this host remembers about the agents it started: socket, pid, token
//! and versions per serial. The token proves that an agent answering on a
//! socket is the one this host started, so only a recorded agent is reused;
//! any other agent is another holder of UI automation.
//!
//! Records live in memory, and also in a directory when the daemon names
//! one, so that a restarted daemon reuses the agent its predecessor left
//! running. Each record is a file readable by its owner only.

use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

/// One agent this host started.
#[derive(Clone, PartialEq, Eq)]
pub struct Record {
    pub serial: String,
    pub socket: String,
    pub pid: u32,
    /// The per-launch token its hello carries.
    pub token: String,
    /// The agent name and version its hello carried.
    pub agent: String,
    /// The protocol version negotiated with it.
    pub protocol: u16,
}

impl fmt::Debug for Record {
    /// Shows whether a token is present, never the token.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Record")
            .field("serial", &self.serial)
            .field("socket", &self.socket)
            .field("pid", &self.pid)
            .field("token", &"<redacted>")
            .field("agent", &self.agent)
            .field("protocol", &self.protocol)
            .finish()
    }
}

impl Record {
    /// The record as the lines of its file: `key=value`.
    pub fn to_text(&self) -> String {
        format!(
            "serial={}\nsocket={}\npid={}\ntoken={}\nagent={}\nprotocol={}\n",
            self.serial, self.socket, self.pid, self.token, self.agent, self.protocol
        )
    }

    /// Parses [`Record::to_text`]; `None` when a field is missing or invalid.
    pub fn from_text(text: &str) -> Option<Record> {
        let fields: HashMap<&str, &str> = text
            .lines()
            .filter_map(|line| line.split_once('='))
            .collect();
        let get = |key: &str| fields.get(key).copied().filter(|v| !v.is_empty());
        Some(Record {
            serial: get("serial")?.to_owned(),
            socket: get("socket")?.to_owned(),
            pid: get("pid")?.parse().ok()?,
            token: get("token")?.to_owned(),
            agent: get("agent")?.to_owned(),
            protocol: get("protocol")?.parse().ok()?,
        })
    }
}

/// The records of one host, shared by every clone.
#[derive(Clone, Debug)]
pub struct Records {
    memory: Arc<Mutex<HashMap<String, Record>>>,
    dir: Option<PathBuf>,
}

impl Records {
    /// Records that last as long as this process.
    pub fn in_memory() -> Records {
        Records {
            memory: Arc::new(Mutex::new(HashMap::new())),
            dir: None,
        }
    }

    /// Records also kept as files in `dir`, created readable by its owner
    /// only when missing.
    pub fn in_dir(dir: &Path) -> io::Result<Records> {
        if !dir.is_dir() {
            fs::create_dir_all(dir)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
            }
        }
        Ok(Records {
            memory: Arc::new(Mutex::new(HashMap::new())),
            dir: Some(dir.to_owned()),
        })
    }

    fn memory(&self) -> MutexGuard<'_, HashMap<String, Record>> {
        // A panic while holding the map leaves it consistent: every
        // operation is a single insert or remove.
        self.memory.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn file(&self, serial: &str) -> Option<PathBuf> {
        self.dir
            .as_ref()
            .map(|dir| dir.join(Records::file_name(serial)))
    }

    /// The directory, when records are kept in one.
    pub fn dir(&self) -> Option<&Path> {
        self.dir.as_deref()
    }

    /// The record for `serial`: the one in memory, else the one in the
    /// directory.
    pub fn get(&self, serial: &str) -> Option<Record> {
        if let Some(record) = self.memory().get(serial) {
            return Some(record.clone());
        }
        let text = fs::read_to_string(self.file(serial)?).ok()?;
        let record = Record::from_text(&text).filter(|r| r.serial == serial)?;
        self.memory().insert(serial.to_owned(), record.clone());
        Some(record)
    }

    /// Stores `record`, replacing any record for its serial.
    pub fn put(&self, record: Record) -> io::Result<()> {
        if let Some(path) = self.file(&record.serial) {
            // Written aside and renamed, so a reader never sees half a record.
            let partial = path.with_extension("agent.partial");
            let mut options = fs::OpenOptions::new();
            options.write(true).create(true).truncate(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&partial)?;
            file.write_all(record.to_text().as_bytes())?;
            file.sync_all()?;
            fs::rename(&partial, &path)?;
        }
        self.memory().insert(record.serial.clone(), record);
        Ok(())
    }

    /// Forgets the record for `serial`.
    pub fn remove(&self, serial: &str) -> io::Result<()> {
        self.memory().remove(serial);
        match self.file(serial).map(fs::remove_file) {
            Some(Err(e)) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }

    /// The file name of `serial`'s record: the serial with every character
    /// other than letters, digits, `.`, `_` and `-` replaced, then `.agent`.
    pub fn file_name(serial: &str) -> String {
        let safe: String = serial
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || "._-".contains(c) {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        format!("{safe}.agent")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(serial: &str, pid: u32) -> Record {
        Record {
            serial: serial.into(),
            socket: "mobile-agent-0.1.0-p1-00ff".into(),
            pid,
            token: "0123456789abcdef".into(),
            agent: "mobile-agent-android 0.1.0".into(),
            protocol: 1,
        }
    }

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "android-records-{tag}-{}-{}",
            std::process::id(),
            random_suffix()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn random_suffix() -> u64 {
        use std::hash::{BuildHasher, Hasher};
        std::collections::hash_map::RandomState::new()
            .build_hasher()
            .finish()
    }

    #[test]
    fn a_record_round_trips_through_its_text() {
        let r = record("emulator-5554", 812);
        assert_eq!(Record::from_text(&r.to_text()), Some(r.clone()));
        assert_eq!(Record::from_text("serial=x\nsocket=y\n"), None);
        assert_eq!(
            Record::from_text(&r.to_text().replace("pid=812", "pid=x")),
            None
        );
    }

    #[test]
    fn debug_output_never_shows_the_token() {
        let text = format!("{:?}", record("emulator-5554", 1));
        assert!(!text.contains("0123456789abcdef"), "{text}");
        assert!(text.contains("<redacted>"), "{text}");
    }

    #[test]
    fn records_in_memory_are_shared_by_clones_and_forgotten_on_remove() {
        let records = Records::in_memory();
        let clone = records.clone();
        records.put(record("emulator-5554", 812)).unwrap();
        assert_eq!(clone.get("emulator-5554").map(|r| r.pid), Some(812));
        assert_eq!(clone.get("emulator-5556"), None);
        clone.put(record("emulator-5554", 900)).unwrap();
        assert_eq!(records.get("emulator-5554").map(|r| r.pid), Some(900));
        records.remove("emulator-5554").unwrap();
        assert_eq!(clone.get("emulator-5554"), None);
        assert!(records.dir().is_none());
    }

    #[test]
    fn records_in_a_directory_outlive_the_process_that_wrote_them() {
        let dir = scratch("persist");
        let first = Records::in_dir(&dir).unwrap();
        first.put(record("192.168.1.5:5555", 812)).unwrap();
        // A restarted daemon: nothing in memory, the file remains.
        let second = Records::in_dir(&dir).unwrap();
        assert_eq!(
            second.get("192.168.1.5:5555"),
            Some(record("192.168.1.5:5555", 812))
        );
        second.remove("192.168.1.5:5555").unwrap();
        assert_eq!(Records::in_dir(&dir).unwrap().get("192.168.1.5:5555"), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn record_files_are_readable_by_their_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("mode");
        let records = Records::in_dir(&dir).unwrap();
        records.put(record("emulator-5554", 812)).unwrap();
        let file = dir.join(Records::file_name("emulator-5554"));
        let mode = std::fs::metadata(&file).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let dir_mode = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(dir_mode, 0o700);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_corrupt_record_file_is_ignored() {
        let dir = scratch("corrupt");
        let records = Records::in_dir(&dir).unwrap();
        std::fs::write(dir.join(Records::file_name("emulator-5554")), "garbage").unwrap();
        assert_eq!(records.get("emulator-5554"), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn serials_become_safe_file_names() {
        assert_eq!(Records::file_name("emulator-5554"), "emulator-5554.agent");
        assert_eq!(
            Records::file_name("192.168.1.5:5555"),
            "192.168.1.5_5555.agent"
        );
        assert_eq!(Records::file_name("../x/y"), ".._x_y.agent");
    }
}
