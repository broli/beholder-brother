use flate2::Compression;
use flate2::write::GzEncoder;
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use time::Date;

use crate::{ArchiveType, Config, Result, parse_date_str};

const HISTORY_FILE: &str = "history.log";
const HISTORY_HEADER: &str = "# date metric min avg max p95";
const METRIC_CPU: u8 = 0b001;
const METRIC_MEM: u8 = 0b010;
const METRIC_NET_DOWN: u8 = 0b100;
const REQUIRED_METRICS_MASK: u8 = METRIC_CPU | METRIC_MEM | METRIC_NET_DOWN;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LogKind {
    Raw,
    Disks,
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

pub fn rollup_and_retire(config: &Config, today: &Date) -> Result<()> {
    let logs = collect_past_logs(&config.log_dir, *today)?;
    let history_path = config.log_dir.join(HISTORY_FILE);
    let mut rolled_dates = prepare_history_state(&history_path)?;
    let mut empty_raw_dates = HashSet::new();

    for log in &logs {
        if log.kind == LogKind::Raw && !rolled_dates.contains(&log.date) {
            match summarize_raw_log(&log.path, log.date, config.interval_ms)? {
                Some(rollup) => {
                    append_rollup_to_history(&history_path, &rollup)?;
                    rolled_dates.insert(log.date);
                    crate::debug_log!("Rolled up {} into {}", log.date, HISTORY_FILE);
                }
                None => {
                    empty_raw_dates.insert(log.date);
                }
            }
        }
    }

    prune_history(&history_path, *today, config.history_days)?;

    let retain_days = i64::try_from(config.retain_days).unwrap_or(i64::MAX);
    for log in &logs {
        if (*today - log.date).whole_days() <= retain_days {
            continue;
        }
        if log.kind == LogKind::Raw
            && !rolled_dates.contains(&log.date)
            && !empty_raw_dates.contains(&log.date)
        {
            continue;
        }
        if let Err(err) = retire_log(&log.path, &config.archive_dir, &config.archive_type) {
            eprintln!("Warning: failed to retire {}: {err}", log.path.display());
        } else {
            crate::debug_log!("Retired {}", log.path.display());
        }
    }

    Ok(())
}

fn collect_past_logs(log_dir: &Path, today: Date) -> io::Result<Vec<DatedLog>> {
    fs::create_dir_all(log_dir)?;
    let inventory = crate::scan_log_dir(log_dir)?;

    let mut logs: Vec<DatedLog> = inventory
        .raw_days
        .into_iter()
        .map(|(date, path)| DatedLog {
            date,
            path,
            kind: LogKind::Raw,
        })
        .chain(
            inventory
                .disk_days
                .into_iter()
                .map(|(date, path)| DatedLog {
                    date,
                    path,
                    kind: LogKind::Disks,
                }),
        )
        .filter(|log| log.date < today)
        .collect();
    logs.sort_by_key(|log| log.date);
    Ok(logs)
}

fn summarize_raw_log(
    path: &Path,
    date: Date,
    fallback_interval_ms: u64,
) -> io::Result<Option<DailyRollup>> {
    let reader = BufReader::new(File::open(path)?);

    let mut cpus = Vec::new();
    let mut mems = Vec::new();
    let mut temps = Vec::new();
    let mut down_ms: u64 = 0;

    for line_res in reader.lines() {
        let line = line_res?;
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some(pt) = crate::parse_raw_line(trimmed, fallback_interval_ms) else {
            continue;
        };

        cpus.push(pt.cpu_pct);
        mems.push(pt.mem_pct);
        if let Some(temp) = pt.temp_c {
            temps.push(temp);
        }
        if !pt.net_ok {
            down_ms = down_ms.saturating_add(pt.interval_ms);
        }
    }

    let Some(cpu_summary) = compute_summary(&mut cpus) else {
        return Ok(None);
    };
    let Some(mem_summary) = compute_summary(&mut mems) else {
        return Ok(None);
    };
    let temp_summary = compute_summary(&mut temps);
    let net_down_sec = down_ms / 1000;

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

fn metric_bit(metric: &str) -> u8 {
    match metric {
        "cpu_pct" => METRIC_CPU,
        "mem_pct" => METRIC_MEM,
        "net_down_sec" => METRIC_NET_DOWN,
        _ => 0,
    }
}

/// Returns dates with complete required metrics in `history_path`,
/// stripping any partial/interrupted daily entries before new rollups append.
fn prepare_history_state(history_path: &Path) -> io::Result<HashSet<Date>> {
    let Some(reader) = crate::open_if_exists(history_path)? else {
        return Ok(HashSet::new());
    };

    let mut lines = Vec::new();
    let mut masks: HashMap<Date, u8> = HashMap::new();

    for line_res in reader.lines() {
        let line = line_res?;
        let trimmed = line.trim();
        if !trimmed.is_empty() && !trimmed.starts_with('#') {
            let mut cols = trimmed.split_whitespace();
            if let (Some(date_str), Some(metric)) = (cols.next(), cols.next())
                && let Some(date) = parse_date_str(date_str)
            {
                let bit = metric_bit(metric);
                *masks.entry(date).or_insert(0) |= bit;
            }
        }
        lines.push(line);
    }

    let mut complete_dates = HashSet::new();
    let mut incomplete_dates = HashSet::new();
    for (date, mask) in masks {
        if (mask & REQUIRED_METRICS_MASK) == REQUIRED_METRICS_MASK {
            complete_dates.insert(date);
        } else {
            incomplete_dates.insert(date);
        }
    }

    if !incomplete_dates.is_empty() {
        let kept_lines: Vec<String> = lines
            .into_iter()
            .filter(|line| {
                let trimmed = line.trim();
                if trimmed.is_empty() || trimmed.starts_with('#') {
                    return true;
                }
                trimmed
                    .split_whitespace()
                    .next()
                    .and_then(parse_date_str)
                    .is_none_or(|d| !incomplete_dates.contains(&d))
            })
            .collect();
        rewrite_lines_atomically(history_path, &kept_lines)?;
    }

    Ok(complete_dates)
}

fn append_rollup_to_history(history_path: &Path, rollup: &DailyRollup) -> io::Result<()> {
    if let Some(parent) = history_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(history_path)?;

    let mut buf = String::new();
    if file.metadata()?.len() == 0 {
        let _ = writeln!(buf, "{HISTORY_HEADER}");
    }

    let date_str = rollup.date;
    let _ = writeln!(
        buf,
        "{date_str} cpu_pct {:.2} {:.2} {:.2} {:.2}",
        rollup.cpu.min, rollup.cpu.avg, rollup.cpu.max, rollup.cpu.p95
    );
    let _ = writeln!(
        buf,
        "{date_str} mem_pct {:.2} {:.2} {:.2} {:.2}",
        rollup.mem.min, rollup.mem.avg, rollup.mem.max, rollup.mem.p95
    );
    if let Some(temp) = &rollup.temp {
        let _ = writeln!(
            buf,
            "{date_str} temp_c {:.1} {:.1} {:.1} {:.1}",
            temp.min, temp.avg, temp.max, temp.p95
        );
    }
    let _ = writeln!(
        buf,
        "{date_str} net_down_sec {} -- -- --",
        rollup.net_down_sec
    );

    file.write_all(buf.as_bytes())?;

    Ok(())
}

fn prune_history(history_path: &Path, today: Date, history_days: u64) -> io::Result<()> {
    let Some(reader) = crate::open_if_exists(history_path)? else {
        return Ok(());
    };

    let max_age = i64::try_from(history_days).unwrap_or(i64::MAX);
    let mut kept_lines = Vec::new();
    let mut removed_any = false;

    for line_res in reader.lines() {
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

    rewrite_lines_atomically(history_path, &kept_lines)
}

fn tmp_path_for(target: &Path) -> io::Result<PathBuf> {
    let mut name = target
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing file name"))?
        .to_os_string();
    name.push(".tmp");
    Ok(target.with_file_name(name))
}

fn rewrite_lines_atomically(target: &Path, lines: &[String]) -> io::Result<()> {
    let tmp_path = tmp_path_for(target)?;
    let write_res = (|| {
        let mut writer = BufWriter::new(File::create(&tmp_path)?);
        for line in lines {
            writeln!(writer, "{line}")?;
        }
        writer.flush()
    })();
    if let Err(err) = write_res {
        let _ = fs::remove_file(&tmp_path);
        return Err(err);
    }
    fs::rename(&tmp_path, target)
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
    let tmp_gz = tmp_path_for(dst_gz)?;
    let mut input = BufReader::new(File::open(src)?);
    let out_file = File::create(&tmp_gz)?;
    let mut encoder = GzEncoder::new(BufWriter::new(out_file), Compression::default());

    if let Err(err) = io::copy(&mut input, &mut encoder)
        .and_then(|_| encoder.finish())
        .and_then(|mut writer| writer.flush())
    {
        let _ = fs::remove_file(&tmp_gz);
        return Err(err);
    }

    fs::rename(&tmp_gz, dst_gz)?;
    fs::remove_file(src)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestTempDir;
    use flate2::read::GzDecoder;
    use std::io::Read as _;
    use std::net::SocketAddr;
    use time::macros::date;

    fn extract_fixture_to(dir: &Path) -> Result<()> {
        let archive =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/two_weeks_logs.tar.gz");
        let mut decoder = GzDecoder::new(File::open(&archive)?);
        let mut hdr = [0u8; 512];
        loop {
            match decoder.read_exact(&mut hdr) {
                Ok(()) => {}
                Err(err) if err.kind() == io::ErrorKind::UnexpectedEof => break,
                Err(err) => return Err(err.into()),
            }
            if hdr.iter().all(|&b| b == 0) {
                break;
            }
            let name_end = hdr[..100].iter().position(|&b| b == 0).unwrap_or(100);
            let raw_name = std::str::from_utf8(&hdr[..name_end])
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            let size_str = std::str::from_utf8(&hdr[124..136])
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?
                .trim_matches(|c: char| c == '\0' || c.is_ascii_whitespace());
            let size = usize::from_str_radix(size_str, 8)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

            let rel = raw_name.trim_start_matches("./");
            let is_file = matches!(hdr[156], b'0' | 0);
            if is_file && !rel.is_empty() && !rel.contains("..") {
                let mut content = vec![0u8; size];
                decoder.read_exact(&mut content)?;
                fs::write(dir.join(rel), &content)?;
            } else if size > 0 {
                let mut skip = vec![0u8; size];
                decoder.read_exact(&mut skip)?;
            }

            let pad = (512 - (size % 512)) % 512;
            if pad > 0 {
                let mut skip = [0u8; 512];
                decoder.read_exact(&mut skip[..pad])?;
            }
        }
        Ok(())
    }

    #[test]
    fn test_rotate_and_idempotency() -> Result<()> {
        let work_dir = TestTempDir::new("rotate")?;
        extract_fixture_to(work_dir.path())?;

        let config = Config {
            interval_ms: 10_000,
            log_dir: work_dir.path().to_path_buf(),
            retain_days: 7,
            history_days: 10,
            archive_dir: work_dir.path().join("archive"),
            archive_type: ArchiveType::Gzip,
            network_target: SocketAddr::from(([1, 1, 1, 1], 53)),
            mounts: vec![PathBuf::from("/")],
            ..Config::default()
        };

        let today = date!(2026 - 11 - 15);
        rollup_and_retire(&config, &today)?;

        // Days 01..=07 (7 raw + 7 disks = 14 files) are > 7 days old and should be gzip-archived.
        let archived_count = fs::read_dir(&config.archive_dir)?.count();
        assert_eq!(archived_count, 14);
        assert!(config.archive_dir.join("raw-2026-11-01.log.gz").exists());
        assert!(!work_dir.path().join("raw-2026-11-01.log").exists());

        // Days 08..=14 (7 raw + 7 disks = 14 files) should remain in log_dir.
        assert!(work_dir.path().join("raw-2026-11-08.log").exists());
        assert!(work_dir.path().join("disks-2026-11-14.log").exists());

        // history.log should retain only days within history_days (05..=14 => 10 days * 4 lines + header).
        let history_first = fs::read_to_string(work_dir.path().join(HISTORY_FILE))?;
        let line_count = history_first.lines().count();
        assert_eq!(line_count, 1 + 10 * 4);
        assert!(!history_first.contains("2026-11-04"));
        assert!(history_first.contains("2026-11-05"));
        assert!(history_first.contains("2026-11-14"));

        // Running rollup_and_retire again on the same day must be idempotent.
        rollup_and_retire(&config, &today)?;
        let history_second = fs::read_to_string(work_dir.path().join(HISTORY_FILE))?;
        assert_eq!(history_first, history_second);

        Ok(())
    }

    #[test]
    fn test_collect_past_logs_from_fixture() -> Result<()> {
        let work_dir = TestTempDir::new("collect_past_logs")?;
        extract_fixture_to(work_dir.path())?;

        let logs = collect_past_logs(work_dir.path(), date!(2026 - 11 - 15))?;
        assert_eq!(logs.len(), 28);
        assert_eq!(logs.first().map(|l| l.date), Some(date!(2026 - 11 - 01)));
        assert_eq!(logs.last().map(|l| l.date), Some(date!(2026 - 11 - 14)));

        Ok(())
    }

    #[test]
    fn test_summarize_raw_log_and_missing_temp() -> Result<()> {
        let work_dir = TestTempDir::new("summarize")?;
        let log_path = work_dir.path().join("raw-2026-11-10.log");
        fs::write(
            &log_path,
            "# timestamp cpu_pct mem_pct temp_c net_ok interval_ms\n\
             1000 10.0 50.0 - 1 10000\n\
             1010 20.0 60.0 - 0 10000\n\
             1012 30.0 70.0 - 0 2000\n\
             1022 40.0 80.0 - 1 1000\n",
        )?;

        // Fallback interval is 1000ms, but recorded per-sample intervals (10000 + 2000) must apply.
        let rollup = summarize_raw_log(&log_path, date!(2026 - 11 - 10), 1_000)?
            .expect("expected non-empty rollup");

        assert_eq!(rollup.cpu.min, 10.0);
        assert_eq!(rollup.cpu.avg, 25.0);
        assert_eq!(rollup.cpu.max, 40.0);
        assert_eq!(rollup.cpu.p95, 40.0);
        assert_eq!(rollup.mem.min, 50.0);
        assert_eq!(rollup.temp, None);
        assert_eq!(rollup.net_down_sec, 12);

        Ok(())
    }

    #[test]
    fn test_incomplete_history_entry_is_repaired_on_rollup() -> Result<()> {
        let work_dir = TestTempDir::new("repair_history")?;
        let raw_path = work_dir.path().join("raw-2026-11-01.log");
        fs::write(
            &raw_path,
            "# timestamp cpu_pct mem_pct temp_c net_ok interval_ms\n\
             1000 12.0 40.0 45.0 0 5000\n",
        )?;

        // Simulate a partial crash where only cpu_pct was written for 2026-11-01.
        let history_path = work_dir.path().join(HISTORY_FILE);
        fs::write(
            &history_path,
            "# date metric min avg max p95\n\
             2026-11-01 cpu_pct 12.00 12.00 12.00 12.00\n",
        )?;

        let config = Config {
            interval_ms: 1_000,
            log_dir: work_dir.path().to_path_buf(),
            retain_days: 7,
            history_days: 180,
            archive_dir: work_dir.path().join("archive"),
            archive_type: ArchiveType::None,
            network_target: SocketAddr::from(([1, 1, 1, 1], 53)),
            mounts: vec![PathBuf::from("/")],
            ..Config::default()
        };

        rollup_and_retire(&config, &date!(2026 - 11 - 15))?;

        let history = fs::read_to_string(&history_path)?;
        assert_eq!(history.matches("2026-11-01 cpu_pct").count(), 1);
        assert_eq!(history.matches("2026-11-01 mem_pct").count(), 1);
        assert_eq!(history.matches("2026-11-01 temp_c").count(), 1);
        assert_eq!(history.matches("2026-11-01 net_down_sec 5").count(), 1);
        assert!(!raw_path.exists());

        Ok(())
    }

    #[test]
    fn test_retire_plain_and_none() -> Result<()> {
        let work_dir = TestTempDir::new("retire_modes")?;
        let archive_dir = work_dir.path().join("archive");

        let plain_src = work_dir.path().join("raw-2026-11-01.log");
        fs::write(&plain_src, "hello\n")?;
        retire_log(&plain_src, &archive_dir, &ArchiveType::Plain)?;
        assert!(!plain_src.exists());
        assert_eq!(
            fs::read_to_string(archive_dir.join("raw-2026-11-01.log"))?,
            "hello\n"
        );

        let none_src = work_dir.path().join("raw-2026-11-02.log");
        fs::write(&none_src, "bye\n")?;
        retire_log(&none_src, &archive_dir, &ArchiveType::None)?;
        assert!(!none_src.exists());
        assert!(!archive_dir.join("raw-2026-11-02.log").exists());

        let gz_src = work_dir.path().join("raw-2026-11-03.log");
        fs::write(&gz_src, "compressed payload\n")?;
        retire_log(&gz_src, &archive_dir, &ArchiveType::Gzip)?;
        assert!(!gz_src.exists());
        let mut decoder = GzDecoder::new(File::open(archive_dir.join("raw-2026-11-03.log.gz"))?);
        let mut decoded = String::new();
        decoder.read_to_string(&mut decoded)?;
        assert_eq!(decoded, "compressed payload\n");

        Ok(())
    }
}
