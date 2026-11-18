use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use time::Date;
use time::macros::format_description;

use crate::{ArchiveType, Config};

const HISTORY_FILE: &str = "history.log";
const HISTORY_HEADER: &str = "# date metric min avg max p95";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LogKind {
    Raw,
    Disks,
}

impl LogKind {
    const fn as_str(self) -> &'static str {
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

#[derive(Debug, PartialEq)]
struct MetricSummary {
    min: f32,
    avg: f32,
    max: f32,
    p95: f32,
}

#[derive(Debug, PartialEq)]
struct DailyRollup {
    date: Date,
    cpu: MetricSummary,
    mem: MetricSummary,
    temp: Option<MetricSummary>, // None if every sample had "-" for temp_c
    net_down_sec: u64,
}

pub fn rollup_and_retire(config: &Config, today: &Date) -> Result<(), Box<dyn std::error::Error>> {
    let logs = collect_past_logs(&config.log_dir, *today)?;
    let history_path = config.log_dir.join(HISTORY_FILE);
    let mut rolled_dates = read_rolled_dates(&history_path)?;

    for log in &logs {
        if log.kind == LogKind::Raw
            && !rolled_dates.contains(&log.date)
            && let Some(rollup) = summarize_raw_log(&log.path, log.date, config.interval_ms)?
        {
            append_rollup_to_history(&history_path, &rollup)?;
            rolled_dates.insert(log.date);
            crate::print_debug(&format!("Rolled up {} into {}", log.date, HISTORY_FILE));
        }
    }

    prune_history(&history_path, *today, config.history_days)?;

    let retain_days = i64::try_from(config.retain_days).unwrap_or(i64::MAX);
    for log in &logs {
        if (*today - log.date).whole_days() > retain_days {
            retire_log(&log.path, &config.archive_dir, &config.archive_type)?;
            crate::print_debug(&format!("Retired {}", log.path.display()));
        }
    }

    Ok(())
}

fn parse_date_str(s: &str) -> Option<Date> {
    let time_format = format_description!("[year]-[month]-[day]");
    Date::parse(s, &time_format).ok()
}

fn parse_log_date(filename: &str, kind: LogKind) -> Option<Date> {
    let date_part = filename
        .strip_prefix(kind.as_str())? //you shall not format me damm rust
        .strip_prefix('-')?
        .strip_suffix(".log")?;
    parse_date_str(date_part)
}

fn collect_past_logs(log_dir: &Path, today: Date) -> io::Result<Vec<DatedLog>> {
    fs::create_dir_all(log_dir)?;

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
        logs.push(DatedLog {
            date: date_of_log,
            path: entry.path(),
            kind: log_kind,
        });
    }
    logs.sort_by_key(|log| log.date);
    Ok(logs)
}

fn summarize_raw_log(
    path: &Path,
    date: Date,
    interval_ms: u64,
) -> io::Result<Option<DailyRollup>> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);

    let mut cpus = Vec::new();
    let mut mems = Vec::new();
    let mut temps = Vec::new();
    let mut down_ticks: u64 = 0;

    for line_res in reader.lines() {
        let line = line_res?;
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        let mut cols = trimmed.split_whitespace();
        let Some(_ts) = cols.next() else {
            continue;
        };
        let (Some(cpu_str), Some(mem_str), Some(temp_str), Some(net_str)) =
            (cols.next(), cols.next(), cols.next(), cols.next())
        else {
            continue;
        };

        let (Ok(cpu), Ok(mem), Ok(net)) = (
            cpu_str.parse::<f32>(),
            mem_str.parse::<f32>(),
            net_str.parse::<u8>(),
        ) else {
            continue;
        };
        if !cpu.is_finite() || !mem.is_finite() {
            continue;
        }

        cpus.push(cpu);
        mems.push(mem);
        if temp_str != "-"
            && let Ok(temp) = temp_str.parse::<f32>()
            && temp.is_finite()
        {
            temps.push(temp);
        }
        if net == 0 {
            down_ticks = down_ticks.saturating_add(1);
        }
    }

    let Some(cpu_summary) = compute_summary(&mut cpus) else {
        return Ok(None);
    };
    let Some(mem_summary) = compute_summary(&mut mems) else {
        return Ok(None);
    };
    let temp_summary = compute_summary(&mut temps);
    let net_down_sec = down_ticks.saturating_mul(interval_ms) / 1000;

    Ok(Some(DailyRollup {
        date,
        cpu: cpu_summary,
        mem: mem_summary,
        temp: temp_summary,
        net_down_sec,
    }))
}

fn compute_summary(values: &mut [f32]) -> Option<MetricSummary> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(f32::total_cmp);

    let min = values[0];
    let max = values[values.len() - 1];
    let sum: f64 = values.iter().map(|&v| f64::from(v)).sum();
    let avg = (sum / values.len() as f64) as f32;

    // Nearest-rank 95th percentile on 0-indexed sorted slice.
    let p95_idx = ((values.len() * 95).div_ceil(100))
        .saturating_sub(1)
        .min(values.len() - 1);
    let p95 = values[p95_idx];

    Some(MetricSummary { min, avg, max, p95 })
}

fn read_rolled_dates(history_path: &Path) -> io::Result<HashSet<Date>> {
    let file = match File::open(history_path) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(HashSet::new()),
        Err(e) => return Err(e),
    };

    let mut dates = HashSet::new();
    for line_res in BufReader::new(file).lines() {
        let line = line_res?;
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some(date_str) = trimmed.split_whitespace().next()
            && let Some(date) = parse_date_str(date_str)
        {
            dates.insert(date);
        }
    }
    Ok(dates)
}

fn append_rollup_to_history(history_path: &Path, rollup: &DailyRollup) -> io::Result<()> {
    if let Some(parent) = history_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(history_path)?;

    if file.metadata()?.len() == 0 {
        writeln!(file, "{HISTORY_HEADER}")?;
    }

    let date_str = rollup.date;
    writeln!(
        file,
        "{date_str} cpu_pct {:.2} {:.2} {:.2} {:.2}",
        rollup.cpu.min, rollup.cpu.avg, rollup.cpu.max, rollup.cpu.p95
    )?;
    writeln!(
        file,
        "{date_str} mem_pct {:.2} {:.2} {:.2} {:.2}",
        rollup.mem.min, rollup.mem.avg, rollup.mem.max, rollup.mem.p95
    )?;
    if let Some(temp) = &rollup.temp {
        writeln!(
            file,
            "{date_str} temp_c {:.1} {:.1} {:.1} {:.1}",
            temp.min, temp.avg, temp.max, temp.p95
        )?;
    }
    writeln!(
        file,
        "{date_str} net_down_sec {} -- -- --",
        rollup.net_down_sec
    )?;

    Ok(())
}

fn prune_history(history_path: &Path, today: Date, history_days: u64) -> io::Result<()> {
    let file = match File::open(history_path) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };

    let max_age = i64::try_from(history_days).unwrap_or(i64::MAX);
    let mut kept_lines = Vec::new();
    let mut removed_any = false;

    for line_res in BufReader::new(file).lines() {
        let line = line_res?;
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            kept_lines.push(line);
            continue;
        }
        let is_expired = trimmed
            .split_whitespace()
            .next()
            .and_then(parse_date_str)
            .is_some_and(|date| (today - date).whole_days() > max_age);
        if is_expired {
            removed_any = true;
        } else {
            kept_lines.push(line);
        }
    }

    if !removed_any {
        return Ok(());
    }

    let tmp_path = history_path.with_extension("log.tmp");
    {
        let tmp_file = File::create(&tmp_path)?;
        let mut writer = BufWriter::new(tmp_file);
        for line in kept_lines {
            writeln!(writer, "{line}")?;
        }
        writer.flush()?;
    }
    fs::rename(&tmp_path, history_path)?;

    Ok(())
}

fn retire_log(path: &Path, archive_dir: &Path, archive_type: &ArchiveType) -> io::Result<()> {
    match archive_type {
        ArchiveType::None => fs::remove_file(path),
        ArchiveType::Plain => {
            fs::create_dir_all(archive_dir)?;
            let file_name = path
                .file_name()
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing file name"))?;
            move_file(path, &archive_dir.join(file_name))
        }
        ArchiveType::Gzip => {
            fs::create_dir_all(archive_dir)?;
            let file_name = path
                .file_name()
                .and_then(|n| n.to_str())
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid file name"))?;
            let dst_gz = archive_dir.join(format!("{file_name}.gz"));
            gzip_file(path, &dst_gz)
        }
    }
}

fn move_file(src: &Path, dst: &Path) -> io::Result<()> {
    match fs::rename(src, dst) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::CrossesDevices => {
            fs::copy(src, dst)?;
            fs::remove_file(src)
        }
        Err(err) => Err(err),
    }
}

fn gzip_file(src: &Path, dst_gz: &Path) -> io::Result<()> {
    let tmp_gz = dst_gz.with_extension("gz.tmp");
    let out_file = File::create(&tmp_gz)?;

    let status = Command::new("gzip")
        .arg("-c")
        .arg(src)
        .stdout(out_file)
        .status();

    match status {
        Ok(s) if s.success() => {
            fs::rename(&tmp_gz, dst_gz)?;
            fs::remove_file(src)?;
            Ok(())
        }
        Ok(s) => {
            let _ = fs::remove_file(&tmp_gz);
            Err(io::Error::other(format!("gzip exited with status: {s}")))
        }
        Err(err) => {
            let _ = fs::remove_file(&tmp_gz);
            Err(err)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::SocketAddr;
    use time::macros::date;

    fn extract_fixture_to(dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
        let archive =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/two_weeks_logs.tar.gz");
        let _ = fs::remove_dir_all(dir);
        fs::create_dir_all(dir)?;

        let status = Command::new("tar")
            .arg("-xzf")
            .arg(&archive)
            .arg("-C")
            .arg(dir)
            .status()?;
        assert!(status.success(), "tar failed with status: {status}");
        Ok(())
    }

    #[test]
    fn test_rotate_and_idempotency() -> Result<(), Box<dyn std::error::Error>> {
        let work_dir = std::env::temp_dir().join("beholder_test_rotate");
        extract_fixture_to(&work_dir)?;

        let config = Config {
            interval_ms: 10_000,
            log_dir: work_dir.clone(),
            retain_days: 7,
            history_days: 10,
            archive_dir: work_dir.join("archive"),
            archive_type: ArchiveType::Gzip,
            network_target: SocketAddr::from(([1, 1, 1, 1], 53)),
            mounts: vec![PathBuf::from("/")],
        };

        let today = date!(2026 - 11 - 15);
        rollup_and_retire(&config, &today)?;

        // Days 01..=07 (7 raw + 7 disks = 14 files) are > 7 days old and should be gzip-archived.
        let archived_count = fs::read_dir(&config.archive_dir)?.count();
        assert_eq!(archived_count, 14);
        assert!(config.archive_dir.join("raw-2026-11-01.log.gz").exists());
        assert!(!work_dir.join("raw-2026-11-01.log").exists());

        // Days 08..=14 (7 raw + 7 disks = 14 files) should remain in log_dir.
        assert!(work_dir.join("raw-2026-11-08.log").exists());
        assert!(work_dir.join("disks-2026-11-14.log").exists());

        // history.log should retain only days within history_days (05..=14 => 10 days * 4 lines + header).
        let history_first = fs::read_to_string(work_dir.join(HISTORY_FILE))?;
        let line_count = history_first.lines().count();
        assert_eq!(line_count, 1 + 10 * 4);
        assert!(!history_first.contains("2026-11-04"));
        assert!(history_first.contains("2026-11-05"));
        assert!(history_first.contains("2026-11-14"));

        // Running rollup_and_retire again on the same day must be idempotent.
        rollup_and_retire(&config, &today)?;
        let history_second = fs::read_to_string(work_dir.join(HISTORY_FILE))?;
        assert_eq!(history_first, history_second);

        let _ = fs::remove_dir_all(&work_dir);
        Ok(())
    }

    #[test]
    fn test_collect_past_logs_from_fixture() -> Result<(), Box<dyn std::error::Error>> {
        let work_dir = std::env::temp_dir().join("beholder_test_collect_past_logs");
        extract_fixture_to(&work_dir)?;

        let logs = collect_past_logs(&work_dir, date!(2026 - 11 - 15))?;
        assert_eq!(logs.len(), 28);
        assert_eq!(logs.first().map(|l| l.date), Some(date!(2026 - 11 - 01)));
        assert_eq!(logs.last().map(|l| l.date), Some(date!(2026 - 11 - 14)));

        let _ = fs::remove_dir_all(&work_dir);
        Ok(())
    }

    #[test]
    fn test_summarize_raw_log_and_missing_temp() -> Result<(), Box<dyn std::error::Error>> {
        let work_dir = std::env::temp_dir().join("beholder_test_summarize");
        let _ = fs::remove_dir_all(&work_dir);
        fs::create_dir_all(&work_dir)?;

        let log_path = work_dir.join("raw-2026-11-10.log");
        fs::write(
            &log_path,
            "# timestamp cpu_pct mem_pct temp_c net_ok\n\
             1000 10.0 50.0 - 1\n\
             1010 20.0 60.0 - 0\n\
             1020 30.0 70.0 - 0\n\
             1030 40.0 80.0 - 1\n",
        )?;

        let rollup = summarize_raw_log(&log_path, date!(2026 - 11 - 10), 10_000)?
            .expect("expected non-empty rollup");

        assert_eq!(rollup.cpu.min, 10.0);
        assert_eq!(rollup.cpu.avg, 25.0);
        assert_eq!(rollup.cpu.max, 40.0);
        assert_eq!(rollup.cpu.p95, 40.0);
        assert_eq!(rollup.mem.min, 50.0);
        assert_eq!(rollup.temp, None);
        assert_eq!(rollup.net_down_sec, 20);

        let _ = fs::remove_dir_all(&work_dir);
        Ok(())
    }

    #[test]
    fn test_retire_plain_and_none() -> Result<(), Box<dyn std::error::Error>> {
        let work_dir = std::env::temp_dir().join("beholder_test_retire_modes");
        let _ = fs::remove_dir_all(&work_dir);
        let archive_dir = work_dir.join("archive");
        fs::create_dir_all(&work_dir)?;

        let plain_src = work_dir.join("raw-2026-11-01.log");
        fs::write(&plain_src, "hello\n")?;
        retire_log(&plain_src, &archive_dir, &ArchiveType::Plain)?;
        assert!(!plain_src.exists());
        assert_eq!(
            fs::read_to_string(archive_dir.join("raw-2026-11-01.log"))?,
            "hello\n"
        );

        let none_src = work_dir.join("raw-2026-11-02.log");
        fs::write(&none_src, "bye\n")?;
        retire_log(&none_src, &archive_dir, &ArchiveType::None)?;
        assert!(!none_src.exists());
        assert!(!archive_dir.join("raw-2026-11-02.log").exists());

        let _ = fs::remove_dir_all(&work_dir);
        Ok(())
    }
}
