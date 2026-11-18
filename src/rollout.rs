use std::fs;
use std::io;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use time::Date;
use time::macros::date;
use time::macros::format_description;

use crate::{ArchiveType, Config};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LogKind {
    Raw,
    Disks,
}

impl LogKind {
    const fn as_str(&self) -> &'static str {
        match self {
            Self::Raw => "raw",
            Self::Disks => "disks",
        }
    }
}

#[derive(Debug)]
struct DatedLog {
    date: Date,
    path: PathBuf,
    kind: LogKind,
}

impl DatedLog {
    fn to_string(&self) -> String {
        format!(
            "date:{}---path{:?}---kind{:?}",
            self.date, self.path, self.kind
        )
    }
}

#[derive(Debug)]
struct MetricSummary {
    min: f32,
    avg: f32,
    max: f32,
    p95: f32,
}

#[derive(Debug)]
struct DailyRollup {
    date: Date,
    cpu: MetricSummary,
    mem: MetricSummary,
    temp: Option<MetricSummary>, // None if every sample had "-" for temp_c
    net_down_sec: u64,
}

pub fn rollup_and_retire(config: &Config, today: &Date) -> Result<(), Box<dyn std::error::Error>> {
    // start of debuging and playstesting
    let logs = collect_past_logs(&config.log_dir, *today)?;
    for log in logs {
        println!("purrty log {:#?}", log);
    }
    // ----------------- END OF DEBUGING AND PLAYTESTING
    /* for old_logs in array {
        let contents = read all file
        compute (contents)
        append to historical

    }
    if time to bye bye
        match config.archive_type {
            ArchiveType::Gzip => compress
            ArchiveType::plain => copy
            ArchiveType::none => delete
        }
    */
    Ok(())
}

fn parse_log_date(filename: &str, kind: LogKind) -> Option<Date> {
    let date_part = filename
        .strip_prefix(kind.as_str())? //you shall not format me damm rust
        .strip_prefix('-')?
        .strip_suffix(".log")?;
    let time_format = format_description!("[year]-[month]-[day]");
    Date::parse(date_part, &time_format).ok()
} //look mama im a programmer! 

fn collect_past_logs(log_dir: &Path, today: Date) -> io::Result<Vec<DatedLog>> {
    fs::create_dir_all(log_dir)?; //just in case. dont judge

    let mut logs: Vec<DatedLog> = Vec::new();
    for entry in fs::read_dir(log_dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let file_name = entry.file_name();
        let Some(filename) = file_name.to_str() else {
            continue;
        };
        let log_kind = if filename.starts_with("raw") {
            LogKind::Raw
        } else if filename.starts_with("disks") {
            LogKind::Disks
        } else {
            continue;
        };
        let Some(date_of_log) = parse_log_date(filename, log_kind) else {
            continue;
        };
        if date_of_log >= today {
            continue;
        }
        let dated_log = DatedLog {
            date: date_of_log,
            path: entry.path(),
            kind: log_kind,
        };
        //#[cfg(debug_assertions)]
        //crate::print_debug(&dated_log.to_string());
        logs.push(dated_log);
    }
    logs.sort_by_key(|log| log.date);
    Ok(logs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rotate() -> Result<(), Box<dyn std::error::Error>> {
        let config = Config {
            interval_ms: 60,
            log_dir: PathBuf::from("/tmp/test/"),
            retain_days: 7,
            history_days: 180,
            archive_dir: PathBuf::from("/tmp/test/archive"),
            archive_type: ArchiveType::Gzip,
            network_target: SocketAddr::from(([1, 1, 1, 1], 53)),
            mounts: vec![PathBuf::from("/")],
        };
        fs::create_dir_all(&config.archive_dir)?;
        //let test_data = test_collect_past_logs_from_fixture()?; NOT NEEDED. ALL TEST FUNCTIONS ARE CALLED
        rollup_and_retire(&config, &date!(2026 - 11 - 15))
    }

    #[test]
    fn test_collect_past_logs_from_fixture() -> Result<(), Box<dyn std::error::Error>> {
        let archive =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/two_weeks_logs.tar.gz");
        let work_dir = Path::new("/tmp/test/");

        let _ = fs::remove_dir_all(work_dir);
        fs::create_dir_all(work_dir)?;

        let status = std::process::Command::new("tar")
            .arg("-xzf")
            .arg(&archive)
            .arg("-C")
            .arg(work_dir)
            .status()?;
        assert!(status.success(), "tar failed with status: {status}");

        let logs = collect_past_logs(work_dir, date!(2026 - 11 - 15))?;
        //println!("{logs:#?}"); enable to show pretty logs

        assert_eq!(logs.len(), 28);
        assert_eq!(logs.first().map(|l| l.date), Some(date!(2026 - 11 - 01)));
        assert_eq!(logs.last().map(|l| l.date), Some(date!(2026 - 11 - 14)));
        Ok(())
    }
}
