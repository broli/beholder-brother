#![forbid(unsafe_code)]

use std::fs::{self, File};
use std::io::{self, BufReader};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use time::format_description::FormatItem;
use time::macros::format_description;
use time::{Date, OffsetDateTime};

pub mod brother;
pub mod error;
pub mod rollout;
pub mod savelog;
pub mod sense;

pub use error::{Error, Result};

pub const DEFAULT_INTERVAL_MS: u64 = 10_000;

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
            interval_ms: DEFAULT_INTERVAL_MS,
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

pub(crate) const DATE_FORMAT: &[FormatItem<'_>] = format_description!("[year]-[month]-[day]");

#[derive(Clone, Debug, PartialEq)]
pub struct RawPoint {
    pub timestamp: u64,
    pub cpu_pct: f32,
    pub mem_pct: f32,
    pub temp_c: Option<f32>,
    pub net_ok: bool,
    pub interval_ms: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LogInventory {
    pub history_path: Option<PathBuf>,
    pub raw_days: Vec<(Date, PathBuf)>,
    pub disk_days: Vec<(Date, PathBuf)>,
}

pub fn scan_log_dir(log_dir: &Path) -> io::Result<LogInventory> {
    let mut inventory = LogInventory::default();

    let entries = match fs::read_dir(log_dir) {
        Ok(rd) => rd,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(inventory),
        Err(err) => return Err(err),
    };

    for entry in entries {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };

        if name == "history.log" {
            inventory.history_path = Some(path);
        } else if let Some(date) = parse_prefixed_log_date(name, "raw") {
            inventory.raw_days.push((date, path));
        } else if let Some(date) = parse_prefixed_log_date(name, "disks") {
            inventory.disk_days.push((date, path));
        }
    }

    inventory.raw_days.sort_by_key(|(date, _)| *date);
    inventory.disk_days.sort_by_key(|(date, _)| *date);
    Ok(inventory)
}

pub(crate) fn open_if_exists(path: &Path) -> io::Result<Option<BufReader<File>>> {
    match File::open(path) {
        Ok(file) => Ok(Some(BufReader::new(file))),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err),
    }
}

pub(crate) fn timestamp_to_utc_date(timestamp: u64) -> Result<Date> {
    let ts = i64::try_from(timestamp).map_err(|_| Error::Parse("timestamp overflow".into()))?;
    Ok(OffsetDateTime::from_unix_timestamp(ts)?.date())
}

pub fn today_utc() -> Result<Date> {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    timestamp_to_utc_date(secs)
}

pub(crate) fn parse_date_str(s: &str) -> Option<Date> {
    Date::parse(s, DATE_FORMAT).ok()
}

pub(crate) fn parse_prefixed_log_date(filename: &str, prefix: &str) -> Option<Date> {
    let date_part = filename
        .strip_prefix(prefix)?
        .strip_prefix('-')?
        .strip_suffix(".log")?;
    parse_date_str(date_part)
}

pub(crate) fn parse_raw_line(line: &str, fallback_interval_ms: u64) -> Option<RawPoint> {
    let mut cols = line.split_whitespace();
    let (Some(ts_s), Some(cpu_s), Some(mem_s), Some(temp_s), Some(net_s)) = (
        cols.next(),
        cols.next(),
        cols.next(),
        cols.next(),
        cols.next(),
    ) else {
        return None;
    };

    let (Ok(timestamp), Ok(cpu_pct), Ok(mem_pct), Ok(net_raw)) = (
        ts_s.parse::<u64>(),
        cpu_s.parse::<f32>(),
        mem_s.parse::<f32>(),
        net_s.parse::<u8>(),
    ) else {
        return None;
    };

    if !(cpu_pct.is_finite() && mem_pct.is_finite()) {
        return None;
    }

    let temp_c = if temp_s == "-" {
        None
    } else {
        let t = temp_s.parse::<f32>().ok()?;
        t.is_finite().then_some(t)
    };

    let interval_ms = cols
        .next()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(fallback_interval_ms);

    Some(RawPoint {
        timestamp,
        cpu_pct,
        mem_pct,
        temp_c,
        net_ok: net_raw != 0,
        interval_ms,
    })
}

#[macro_export]
macro_rules! debug_log {
    ($($arg:tt)*) => {{
        if cfg!(debug_assertions) {
            eprintln!("[debug] {}", format_args!($($arg)*));
        }
    }};
}

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
