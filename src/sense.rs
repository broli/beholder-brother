use std::fs;
use std::net::TcpStream;
use std::path::PathBuf;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{Config, Error, Result};

#[derive(Debug)]
pub struct Sample {
    pub timestamp: u64,
    pub cpu_pct: f32,
    pub mem_pct: f32,
    pub temp_c: Option<f32>,
    pub net_ok: bool,
}

#[derive(Debug)]
pub struct DiskSample {
    pub mount: PathBuf,
    pub used_pct: f32,
}

#[derive(Default)]
pub struct PreviousCpuTick {
    pub idle: u64,
    pub total: u64,
    pub timestamp: u64,
}

impl PreviousCpuTick {
    pub fn new() -> Self {
        Self {
            idle: 0,
            total: 0,
            timestamp: 0,
        }
    }
}

pub fn sense(cfg: &Config, prev_cpu: &mut PreviousCpuTick) -> Result<Sample> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let cpu_pct = read_cpu_pct(prev_cpu, timestamp)?;
    let mem_pct = read_mem_pct()?;
    let temp_c = read_temp_c();
    let net_ok = check_network(cfg.network_target);

    Ok(Sample {
        timestamp,
        cpu_pct,
        mem_pct,
        temp_c,
        net_ok,
    })
}

fn read_cpu_pct(prev: &mut PreviousCpuTick, timestamp: u64) -> Result<f32> {
    let content = fs::read_to_string("/proc/stat")?;
    let first_line = content.lines().next().ok_or("Empty /proc/stat")?;
    parse_cpu_stat_line(first_line, prev, timestamp)
}

fn parse_stat_col(col: Option<&str>, name: &str) -> Result<u64> {
    let raw = col.ok_or_else(|| Error::Parse(format!("missing /proc/stat column: {name}")))?;
    raw.parse::<u64>()
        .map_err(|e| Error::Parse(format!("invalid /proc/stat {name} '{raw}': {e}")))
}

fn parse_opt_stat_col(col: Option<&str>, name: &str) -> Result<u64> {
    match col {
        Some(raw) => raw
            .parse::<u64>()
            .map_err(|e| Error::Parse(format!("invalid /proc/stat {name} '{raw}': {e}"))),
        None => Ok(0),
    }
}

fn parse_cpu_stat_line(line: &str, prev: &mut PreviousCpuTick, timestamp: u64) -> Result<f32> {
    let mut parts = line.split_whitespace();

    match parts.next() {
        Some("cpu") => {}
        other => {
            return Err(Error::Parse(format!(
                "expected 'cpu' line in /proc/stat, got {other:?}"
            )));
        }
    }

    let user = parse_stat_col(parts.next(), "user")?;
    let nice = parse_stat_col(parts.next(), "nice")?;
    let system = parse_stat_col(parts.next(), "system")?;
    let idle = parse_stat_col(parts.next(), "idle")?;
    let iowait = parse_opt_stat_col(parts.next(), "iowait")?;
    let irq = parse_opt_stat_col(parts.next(), "irq")?;
    let softirq = parse_opt_stat_col(parts.next(), "softirq")?;
    let steal = parse_opt_stat_col(parts.next(), "steal")?;

    let total_idle = idle + iowait;
    let total = user + nice + system + total_idle + irq + softirq + steal;

    let pct = if prev.total == 0 {
        0.0
    } else {
        let delta_total = total.saturating_sub(prev.total);
        let delta_idle = total_idle.saturating_sub(prev.idle);

        if delta_total == 0 {
            0.0
        } else {
            let delta_busy = delta_total.saturating_sub(delta_idle);
            (delta_busy as f32 / delta_total as f32) * 100.0
        }
    };

    prev.total = total;
    prev.idle = total_idle;
    prev.timestamp = timestamp;

    Ok(pct)
}

fn read_mem_pct() -> Result<f32> {
    let meminfo = fs::read_to_string("/proc/meminfo")?;

    let mut mem_total: Option<u64> = None;
    let mut mem_available: Option<u64> = None;

    for line in meminfo.lines() {
        if line.starts_with("MemTotal:") {
            mem_total = line.split_whitespace().nth(1).and_then(|s| s.parse().ok());
        } else if line.starts_with("MemAvailable:") {
            mem_available = line.split_whitespace().nth(1).and_then(|s| s.parse().ok());
        }
        if mem_total.is_some() && mem_available.is_some() {
            break;
        }
    }

    let total = mem_total.ok_or("MemTotal not found in /proc/meminfo")?;
    let available = mem_available.ok_or("MemAvailable not found in /proc/meminfo")?;

    if total == 0 {
        return Ok(0.0);
    }

    let used = total.saturating_sub(available);
    let pct = (used as f32 / total as f32) * 100.0;

    Ok(pct)
}

pub fn sense_disks(cfg: &Config) -> Result<Vec<DiskSample>> {
    let mut results = Vec::new();

    for path in &cfg.mounts {
        let stat = nix::sys::statvfs::statvfs(path)?;

        let total = stat.blocks();
        let free = stat.blocks_free();
        let avail = stat.blocks_available();

        let used = total.saturating_sub(free);
        let total_user = used + avail;

        let pct = if total_user == 0 {
            0.0
        } else {
            (used as f32 / total_user as f32) * 100.0
        };

        results.push(DiskSample {
            mount: path.clone(),
            used_pct: pct,
        });
    }

    Ok(results)
}

fn read_temp_c() -> Option<f32> {
    let raw = fs::read_to_string("/sys/class/thermal/thermal_zone0/temp").ok()?;
    let milli: f32 = raw.trim().parse().ok()?;
    Some(milli / 1000.0)
}

fn check_network(target: std::net::SocketAddr) -> bool {
    TcpStream::connect_timeout(&target, Duration::from_millis(500)).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_cpu_stat_line_accepts_4_and_8_columns() -> Result<()> {
        let mut prev = PreviousCpuTick::default();
        assert_eq!(parse_cpu_stat_line("cpu 100 0 50 850", &mut prev, 10)?, 0.0);
        let pct = parse_cpu_stat_line("cpu 150 0 100 950 0 0 0 0", &mut prev, 20)?;
        assert!((pct - 50.0).abs() < 0.01);
        Ok(())
    }

    #[test]
    fn test_parse_cpu_stat_line_rejects_malformed_or_truncated() {
        let mut prev = PreviousCpuTick::default();
        assert!(parse_cpu_stat_line("intr 100 0 50 850", &mut prev, 10).is_err());
        assert!(parse_cpu_stat_line("cpu 100 0 50", &mut prev, 10).is_err());
        assert!(parse_cpu_stat_line("cpu 100 0 50 850 bad", &mut prev, 10).is_err());
    }
}
