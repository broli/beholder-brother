use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use time::{Date, OffsetDateTime};

pub mod brother;
pub mod rollout;
pub mod savelog;
pub mod sense;

pub struct Config {
    pub interval_ms: u64,
    pub log_dir: PathBuf,
    pub retain_days: u64,
    pub history_days: u64,
    pub archive_dir: PathBuf,
    pub archive_type: ArchiveType,
    pub network_target: SocketAddr,
    pub mounts: Vec<PathBuf>,
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

pub fn return_today() -> Result<Date, Box<dyn std::error::Error>> {
    Ok(OffsetDateTime::from_unix_timestamp(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64,
    )?
    .date())
}

#[cfg(debug_assertions)]
pub fn print_debug(msg: &str) {
    eprintln!("[debug] {msg}");
}

#[cfg(not(debug_assertions))]
pub fn print_debug(_msg: &str) {}
