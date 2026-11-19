use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use time::Date;
use time::macros::format_description;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileStamp {
    pub modified: Option<SystemTime>,
    pub len: u64,
}

impl FileStamp {
    /// Reads file metadata without opening or mutating the file's contents.
    pub fn from_path(path: &Path) -> io::Result<Option<Self>> {
        match fs::metadata(path) {
            Ok(meta) => Ok(Some(Self {
                modified: meta.modified().ok(),
                len: meta.len(),
            })),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(err) => Err(err),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MetricStats {
    pub min: f32,
    pub avg: f32,
    pub max: f32,
    pub p95: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct HistoryDay {
    pub date: Date,
    pub cpu: Option<MetricStats>,
    pub mem: Option<MetricStats>,
    pub temp: Option<MetricStats>,
    pub net_down_sec: Option<u64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RawPoint {
    pub timestamp: u64,
    pub cpu_pct: f32,
    pub mem_pct: f32,
    pub temp_c: Option<f32>,
    pub net_ok: bool,
    pub interval_ms: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DiskPoint {
    pub timestamp: u64,
    pub mount: String,
    pub used_pct: f32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LogInventory {
    pub history_path: Option<PathBuf>,
    pub raw_days: Vec<(Date, PathBuf)>,
    pub disk_days: Vec<(Date, PathBuf)>,
}

fn parse_date_str(s: &str) -> Option<Date> {
    let format = format_description!("[year]-[month]-[day]");
    Date::parse(s, &format).ok()
}

fn parse_prefixed_log_date(filename: &str, prefix: &str) -> Option<Date> {
    let date_part = filename
        .strip_prefix(prefix)?
        .strip_prefix('-')?
        .strip_suffix(".log")?;
    parse_date_str(date_part)
}

/// Scans `log_dir` read-only for `history.log`, `raw-YYYY-MM-DD.log`, and `disks-YYYY-MM-DD.log`.
pub fn scan_log_dir(log_dir: &Path) -> io::Result<LogInventory> {
    let mut inventory = LogInventory::default();

    let entries = match fs::read_dir(log_dir) {
        Ok(rd) => rd,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(inventory),
        Err(err) => return Err(err),
    };

    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
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

/// Parses `history.log` into daily summaries sorted ascending by date.
/// Returns an empty `Vec` if the file does not exist yet.
pub fn parse_history(path: &Path) -> io::Result<Vec<HistoryDay>> {
    let file = match File::open(path) {
        Ok(f) => f,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err),
    };

    let mut days: BTreeMap<Date, HistoryDay> = BTreeMap::new();

    for line_res in BufReader::new(file).lines() {
        let line = line_res?;
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        let mut cols = trimmed.split_whitespace();
        let (Some(date_str), Some(metric)) = (cols.next(), cols.next()) else {
            continue;
        };
        let Some(date) = parse_date_str(date_str) else {
            continue;
        };

        let entry = days.entry(date).or_insert_with(|| HistoryDay {
            date,
            cpu: None,
            mem: None,
            temp: None,
            net_down_sec: None,
        });

        if metric == "net_down_sec" {
            if let Some(sec) = cols.next().and_then(|s| s.parse::<u64>().ok()) {
                entry.net_down_sec = Some(sec);
            }
            continue;
        }

        let (Some(min_s), Some(avg_s), Some(max_s), Some(p95_s)) =
            (cols.next(), cols.next(), cols.next(), cols.next())
        else {
            continue;
        };
        let (Ok(min), Ok(avg), Ok(max), Ok(p95)) = (
            min_s.parse::<f32>(),
            avg_s.parse::<f32>(),
            max_s.parse::<f32>(),
            p95_s.parse::<f32>(),
        ) else {
            continue;
        };
        if !(min.is_finite() && avg.is_finite() && max.is_finite() && p95.is_finite()) {
            continue;
        }

        let stats = MetricStats { min, avg, max, p95 };
        match metric {
            "cpu_pct" => entry.cpu = Some(stats),
            "mem_pct" => entry.mem = Some(stats),
            "temp_c" => entry.temp = Some(stats),
            _ => {}
        }
    }

    Ok(days.into_values().collect())
}

/// Parses a `raw-YYYY-MM-DD.log` file, skipping comments and malformed/partial lines.
pub fn parse_raw_log(path: &Path) -> io::Result<Vec<RawPoint>> {
    let file = match File::open(path) {
        Ok(f) => f,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err),
    };

    let mut points = Vec::new();
    for line_res in BufReader::new(file).lines() {
        let line = line_res?;
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        let mut cols = trimmed.split_whitespace();
        let (Some(ts_s), Some(cpu_s), Some(mem_s), Some(temp_s), Some(net_s), Some(interval_s)) = (
            cols.next(),
            cols.next(),
            cols.next(),
            cols.next(),
            cols.next(),
            cols.next(),
        ) else {
            continue;
        };

        let (Ok(timestamp), Ok(cpu_pct), Ok(mem_pct), Ok(net_raw), Ok(interval_ms)) = (
            ts_s.parse::<u64>(),
            cpu_s.parse::<f32>(),
            mem_s.parse::<f32>(),
            net_s.parse::<u8>(),
            interval_s.parse::<u64>(),
        ) else {
            continue;
        };

        if !(cpu_pct.is_finite() && mem_pct.is_finite()) {
            continue;
        }

        let temp_c = if temp_s == "-" {
            None
        } else {
            match temp_s.parse::<f32>() {
                Ok(t) if t.is_finite() => Some(t),
                _ => continue,
            }
        };

        points.push(RawPoint {
            timestamp,
            cpu_pct,
            mem_pct,
            temp_c,
            net_ok: net_raw != 0,
            interval_ms,
        });
    }

    Ok(points)
}

/// Parses a `disks-YYYY-MM-DD.log` file.
pub fn parse_disks_log(path: &Path) -> io::Result<Vec<DiskPoint>> {
    let file = match File::open(path) {
        Ok(f) => f,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err),
    };

    let mut points = Vec::new();
    for line_res in BufReader::new(file).lines() {
        let line = line_res?;
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        let Some((ts_s, rest)) = trimmed.split_once(char::is_whitespace) else {
            continue;
        };
        let Some((mount_raw, used_s)) = rest.trim_start().rsplit_once(char::is_whitespace) else {
            continue;
        };

        let (Ok(timestamp), Ok(used_pct)) = (ts_s.parse::<u64>(), used_s.trim().parse::<f32>())
        else {
            continue;
        };
        if !used_pct.is_finite() {
            continue;
        }

        points.push(DiskPoint {
            timestamp,
            mount: mount_raw.trim().to_string(),
            used_pct,
        });
    }

    Ok(points)
}

/// Downsamples `points` to at most `max_points` buckets while preserving any network outage (`net_ok == false`).
pub fn downsample_raw(points: &[RawPoint], max_points: usize) -> Vec<RawPoint> {
    if max_points == 0 || points.len() <= max_points {
        return points.to_vec();
    }

    let n = points.len();
    let mut out = Vec::with_capacity(max_points);

    for i in 0..max_points {
        let start = (i * n) / max_points;
        let end = (((i + 1) * n) / max_points).max(start + 1).min(n);
        let bucket = &points[start..end];

        let count = bucket.len() as f64;
        let cpu_sum: f64 = bucket.iter().map(|p| f64::from(p.cpu_pct)).sum();
        let mem_sum: f64 = bucket.iter().map(|p| f64::from(p.mem_pct)).sum();

        let (temp_sum, temp_count) = bucket
            .iter()
            .filter_map(|p| p.temp_c)
            .fold((0.0_f64, 0_usize), |(s, c), t| (s + f64::from(t), c + 1));

        let temp_c = if temp_count > 0 {
            Some((temp_sum / temp_count as f64) as f32)
        } else {
            None
        };

        let mid = &bucket[bucket.len() / 2];
        out.push(RawPoint {
            timestamp: mid.timestamp,
            cpu_pct: (cpu_sum / count) as f32,
            mem_pct: (mem_sum / count) as f32,
            temp_c,
            net_ok: bucket.iter().all(|p| p.net_ok),
            interval_ms: mid.interval_ms,
        });
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};
    use time::macros::date;

    struct TestTempDir(PathBuf);

    impl TestTempDir {
        fn new(label: &str) -> io::Result<Self> {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            let dir = std::env::temp_dir().join(format!(
                "brother_logs_test_{label}_{}_{}",
                std::process::id(),
                nanos
            ));
            fs::create_dir_all(&dir)?;
            Ok(Self(dir))
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestTempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn test_scan_log_dir_orders_dates_and_handles_missing_dir() -> io::Result<()> {
        let missing = Path::new("/tmp/definitely_nonexistent_brother_dir_12345");
        let empty_inv = scan_log_dir(missing)?;
        assert_eq!(empty_inv, LogInventory::default());

        let dir = TestTempDir::new("scan")?;
        fs::write(dir.path().join("history.log"), "")?;
        fs::write(dir.path().join("raw-2026-11-12.log"), "")?;
        fs::write(dir.path().join("raw-2026-11-10.log"), "")?;
        fs::write(dir.path().join("disks-2026-11-11.log"), "")?;
        fs::write(dir.path().join("notes.txt"), "")?;

        let inv = scan_log_dir(dir.path())?;
        assert!(inv.history_path.is_some());
        assert_eq!(inv.raw_days.len(), 2);
        assert_eq!(inv.raw_days[0].0, date!(2026 - 11 - 10));
        assert_eq!(inv.raw_days[1].0, date!(2026 - 11 - 12));
        assert_eq!(inv.disk_days.len(), 1);
        assert_eq!(inv.disk_days[0].0, date!(2026 - 11 - 11));

        Ok(())
    }

    #[test]
    fn test_parse_history_handles_missing_temp_and_net_dashes() -> io::Result<()> {
        let dir = TestTempDir::new("history")?;
        let path = dir.path().join("history.log");
        fs::write(
            &path,
            "# date metric min avg max p95\n\
             2026-11-11 cpu_pct 1.00 5.00 20.00 15.00\n\
             2026-11-10 cpu_pct 2.00 4.00 10.00 8.00\n\
             2026-11-10 mem_pct 30.00 35.00 40.00 39.00\n\
             2026-11-10 net_down_sec 20 -- -- --\n\
             2026-11-11 mem_pct 31.00 32.00 33.00 33.00\n\
             2026-11-11 temp_c 45.0 48.0 52.0 51.0\n\
             2026-11-11 net_down_sec 0 -- -- --\n\
             corrupted line\n",
        )?;

        let days = parse_history(&path)?;
        assert_eq!(days.len(), 2);
        assert_eq!(days[0].date, date!(2026 - 11 - 10));
        assert_eq!(days[0].temp, None);
        assert_eq!(days[0].net_down_sec, Some(20));

        assert_eq!(days[1].date, date!(2026 - 11 - 11));
        assert_eq!(
            days[1].temp,
            Some(MetricStats {
                min: 45.0,
                avg: 48.0,
                max: 52.0,
                p95: 51.0
            })
        );
        assert_eq!(days[1].net_down_sec, Some(0));

        Ok(())
    }

    #[test]
    fn test_parse_raw_and_downsample_preserves_outage() -> io::Result<()> {
        let dir = TestTempDir::new("raw")?;
        let path = dir.path().join("raw-2026-11-10.log");
        fs::write(
            &path,
            "# timestamp cpu_pct mem_pct temp_c net_ok interval_ms\n\
             1000 10.00 40.00 - 1 10000\n\
             1010 20.00 42.00 50.0 0 10000\n\
             1020 30.00 44.00 52.0 1 10000\n\
             1030 40.00 46.00 54.0 1 10000\n\
             partial_line\n",
        )?;

        let points = parse_raw_log(&path)?;
        assert_eq!(points.len(), 4);
        assert_eq!(points[0].temp_c, None);
        assert!(!points[1].net_ok);

        let downsampled = downsample_raw(&points, 2);
        assert_eq!(downsampled.len(), 2);
        assert!(!downsampled[0].net_ok); // outage preserved in bucket 0
        assert!(downsampled[1].net_ok);
        assert!((downsampled[0].cpu_pct - 15.0).abs() < 0.01);
        assert!((downsampled[1].cpu_pct - 35.0).abs() < 0.01);

        Ok(())
    }

    #[test]
    fn test_parse_disks_log() -> io::Result<()> {
        let dir = TestTempDir::new("disks")?;
        let path = dir.path().join("disks-2026-11-10.log");
        fs::write(
            &path,
            "# timestamp mount used_pct\n\
             1000 / 62.50\n\
             1000 /home 41.25\n",
        )?;

        let disks = parse_disks_log(&path)?;
        assert_eq!(disks.len(), 2);
        assert_eq!(disks[0].mount, "/");
        assert_eq!(disks[0].used_pct, 62.5);
        assert_eq!(disks[1].mount, "/home");

        Ok(())
    }
}
