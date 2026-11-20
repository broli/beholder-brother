#![forbid(unsafe_code)]

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use time::macros::format_description;
use time::{Date, OffsetDateTime};

pub mod brother;
pub mod error;
pub mod rollout;
pub mod savelog;
pub mod sense;

pub use error::{Error, Result};

pub struct Config {
    pub interval_ms: u64,
    pub log_dir: PathBuf,
    pub retain_days: u64,
    pub history_days: u64,
    pub archive_dir: PathBuf,
    pub archive_type: ArchiveType,
    pub network_target: SocketAddr,
    pub mounts: Vec<PathBuf>,
    pub flush_each_sample: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            interval_ms: 10000,
            log_dir: PathBuf::from("/var/log/beholder"),
            retain_days: 7,
            history_days: 180,
            archive_dir: PathBuf::from("/var/log/beholder/archive"),
            archive_type: ArchiveType::Gzip,
            network_target: SocketAddr::from(([1, 1, 1, 1], 53)),
            mounts: vec![PathBuf::from("/")],
            flush_each_sample: false,
        }
    }
}

#[derive(Default)]
pub enum ArchiveType {
    Plain,
    #[default]
    Gzip,
    None,
}

pub fn return_today() -> Result<Date> {
    Ok(OffsetDateTime::from_unix_timestamp(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64,
    )?
    .date())
}

pub(crate) fn parse_date_str(s: &str) -> Option<Date> {
    let format = format_description!("[year]-[month]-[day]");
    Date::parse(s, &format).ok()
}

pub(crate) fn parse_prefixed_log_date(filename: &str, prefix: &str) -> Option<Date> {
    let date_part = filename
        .strip_prefix(prefix)?
        .strip_prefix('-')?
        .strip_suffix(".log")?;
    parse_date_str(date_part)
}

#[cfg(debug_assertions)]
pub fn print_debug(msg: &str) {
    eprintln!("[debug] {msg}");
}

#[cfg(not(debug_assertions))]
pub fn print_debug(_msg: &str) {}

#[cfg(test)]
pub(crate) mod test_support {
    use std::fs;
    use std::io;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

    pub(crate) struct TestTempDir {
        path: PathBuf,
    }

    impl TestTempDir {
        pub(crate) fn new(prefix: &str) -> io::Result<Self> {
            let base = std::env::temp_dir();
            let pid = std::process::id();
            loop {
                let seq = TEST_DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
                let candidate = base.join(format!("beholder_{prefix}_{pid}_{seq}"));
                match fs::create_dir(&candidate) {
                    Ok(()) => return Ok(Self { path: candidate }),
                    Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {}
                    Err(err) => return Err(err),
                }
            }
        }

        pub(crate) fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TestTempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
