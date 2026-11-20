use std::collections::BTreeMap;
use std::fs;
use std::io::BufRead;
use std::path::Path;
use std::time::SystemTime;
use time::Date;

use crate::{DEFAULT_INTERVAL_MS, Result};
pub use crate::{LogInventory, RawPoint, scan_log_dir};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileStamp {
    pub modified: Option<SystemTime>,
    pub len: u64,
}

impl FileStamp {
    pub fn from_path(path: &Path) -> Result<Option<Self>> {
        match fs::metadata(path) {
            Ok(meta) => Ok(Some(Self {
                modified: meta.modified().ok(),
                len: meta.len(),
            })),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(err) => Err(err.into()),
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
pub struct DiskPoint {
    pub timestamp: u64,
    pub mount: String,
    pub used_pct: f32,
}

pub fn parse_history(path: &Path) -> Result<Vec<HistoryDay>> {
    let Some(reader) = crate::open_if_exists(path)? else {
        return Ok(Vec::new());
    };

    let mut days: BTreeMap<Date, HistoryDay> = BTreeMap::new();

    for line_res in reader.lines() {
        let line = line_res?;
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        let mut cols = trimmed.split_whitespace();
        let (Some(date_str), Some(metric)) = (cols.next(), cols.next()) else {
            continue;
        };
        let Some(date) = crate::parse_date_str(date_str) else {
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

pub fn parse_raw_log(path: &Path) -> Result<Vec<RawPoint>> {
    let Some(reader) = crate::open_if_exists(path)? else {
        return Ok(Vec::new());
    };

    let mut points = Vec::new();
    for line_res in reader.lines() {
        let line = line_res?;
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some(pt) = crate::parse_raw_line(trimmed, DEFAULT_INTERVAL_MS) {
            points.push(pt);
        }
    }

    Ok(points)
}

fn parse_disk_line(line: &str) -> Option<(u64, &str, f32)> {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return None;
    }

    let (ts_s, rest) = trimmed.split_once(char::is_whitespace)?;
    let (mount_raw, used_s) = rest.trim_start().rsplit_once(char::is_whitespace)?;
    let (Ok(timestamp), Ok(used_pct)) = (ts_s.parse::<u64>(), used_s.trim().parse::<f32>()) else {
        return None;
    };
    used_pct
        .is_finite()
        .then_some((timestamp, mount_raw.trim(), used_pct))
}

pub fn parse_disks_log(path: &Path) -> Result<Vec<DiskPoint>> {
    let Some(reader) = crate::open_if_exists(path)? else {
        return Ok(Vec::new());
    };

    let mut points = Vec::new();
    for line_res in reader.lines() {
        let line = line_res?;
        if let Some((timestamp, mount, used_pct)) = parse_disk_line(&line) {
            points.push(DiskPoint {
                timestamp,
                mount: mount.to_string(),
                used_pct,
            });
        }
    }

    Ok(points)
}

pub fn parse_latest_disks(path: &Path) -> Result<BTreeMap<String, f32>> {
    let Some(reader) = crate::open_if_exists(path)? else {
        return Ok(BTreeMap::new());
    };

    let mut latest: BTreeMap<String, f32> = BTreeMap::new();
    for line_res in reader.lines() {
        let line = line_res?;
        let Some((_, mount, used_pct)) = parse_disk_line(&line) else {
            continue;
        };
        if let Some(slot) = latest.get_mut(mount) {
            *slot = used_pct;
        } else {
            latest.insert(mount.to_string(), used_pct);
        }
    }

    Ok(latest)
}

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
    use crate::test_support::TestTempDir;
    use time::macros::date;

    #[test]
    fn test_scan_log_dir_orders_dates_and_handles_missing_dir() -> Result<()> {
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
    fn test_parse_history_handles_missing_temp_and_net_dashes() -> Result<()> {
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
    fn test_parse_raw_and_downsample_preserves_outage() -> Result<()> {
        let dir = TestTempDir::new("raw")?;
        let path = dir.path().join("raw-2026-11-10.log");
        fs::write(
            &path,
            "# timestamp cpu_pct mem_pct temp_c net_ok interval_ms\n\
             1000 10.00 40.00 - 1 10000\n\
             1010 20.00 42.00 50.0 0 10000\n\
             1020 30.00 44.00 52.0 1\n\
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
    fn test_parse_disks_log() -> Result<()> {
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
