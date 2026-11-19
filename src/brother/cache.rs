use crate::brother::html::{self, DashboardData, HistoryView, LiveSummary, RetainedDayView};
use crate::brother::logs::{
    DiskPoint, FileStamp, RawPoint, downsample_raw, parse_disks_log, parse_history, parse_raw_log,
    scan_log_dir,
};
use crate::brother::svg::{
    render_disk_bars, render_history_band_chart, render_net_down_chart, render_raw_line_chart,
};
use std::collections::{BTreeMap, HashSet};
use std::io;
use std::path::Path;
use time::Date;

const MAX_RAW_SVG_POINTS: usize = 600;
const SECS_12H: u64 = 12 * 3600;
const SECS_24H: u64 = 24 * 3600;

struct CachedHistory {
    stamp: FileStamp,
    view: HistoryView,
}

struct CachedDay {
    stamp: FileStamp,
    view: RetainedDayView,
}

#[derive(Default)]
pub struct DashboardCache {
    history: Option<CachedHistory>,
    retained_days: BTreeMap<Date, CachedDay>,
}

impl DashboardCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Builds the full 3-tab HTML dashboard, reusing cached SVGs for immutable past days
    /// and unchanged `history.log` unless `force` is true.
    pub fn build_dashboard(
        &mut self,
        log_dir: &Path,
        today: Date,
        force: bool,
    ) -> io::Result<String> {
        if force {
            self.history = None;
            self.retained_days.clear();
        }

        let inventory = scan_log_dir(log_dir)?;

        let history = self.load_history(inventory.history_path.as_deref())?;

        // Evict dates that beholder has retired so memory stays bounded.
        let active_dates: HashSet<Date> = inventory.raw_days.iter().map(|(d, _)| *d).collect();
        self.retained_days.retain(|d, _| active_dates.contains(d));

        let mut retained = Vec::new();
        for (date, path) in &inventory.raw_days {
            if *date < today {
                let day_view = self.load_retained_day(*date, path)?;
                retained.push(day_view);
            }
        }

        let live = build_live_summary(&inventory.raw_days, &inventory.disk_days)?;

        let data = DashboardData {
            log_dir_display: log_dir.display().to_string(),
            live,
            retained,
            history,
        };
        Ok(html::render_page(&data))
    }

    fn load_history(&mut self, history_path: Option<&Path>) -> io::Result<HistoryView> {
        let Some(path) = history_path else {
            self.history = None;
            return Ok(render_history_view(&[]));
        };

        let Some(stamp) = FileStamp::from_path(path)? else {
            self.history = None;
            return Ok(render_history_view(&[]));
        };

        if let Some(cached) = &self.history
            && cached.stamp == stamp
        {
            return Ok(cached.view.clone());
        }

        let days = parse_history(path)?;
        let view = render_history_view(&days);
        self.history = Some(CachedHistory {
            stamp,
            view: view.clone(),
        });
        Ok(view)
    }

    fn load_retained_day(&mut self, date: Date, path: &Path) -> io::Result<RetainedDayView> {
        let stamp = FileStamp::from_path(path)?.unwrap_or(FileStamp {
            modified: None,
            len: 0,
        });

        if let Some(cached) = self.retained_days.get(&date)
            && cached.stamp == stamp
        {
            return Ok(cached.view.clone());
        }

        let raw = parse_raw_log(path)?;
        let samples = raw.len();
        let outages = raw.iter().filter(|p| !p.net_ok).count();
        let downsampled = downsample_raw(&raw, MAX_RAW_SVG_POINTS);
        let (cpu_svg, mem_svg, temp_svg) = render_raw_trio(&downsampled, "Daily");

        let view = RetainedDayView {
            date_label: date.to_string(),
            samples,
            outages,
            cpu_svg,
            mem_svg,
            temp_svg,
        };
        self.retained_days.insert(
            date,
            CachedDay {
                stamp,
                view: view.clone(),
            },
        );
        Ok(view)
    }
}

fn render_history_view(days: &[crate::brother::logs::HistoryDay]) -> HistoryView {
    HistoryView {
        days_count: days.len(),
        cpu_svg: render_history_band_chart(
            "CPU Load History (%)",
            "%",
            "--blue",
            days,
            |d| d.cpu.as_ref(),
            Some((0.0, 100.0)),
        ),
        mem_svg: render_history_band_chart(
            "Memory Usage History (%)",
            "%",
            "--mauve",
            days,
            |d| d.mem.as_ref(),
            Some((0.0, 100.0)),
        ),
        temp_svg: render_history_band_chart(
            "Temperature History (°C)",
            "°C",
            "--peach",
            days,
            |d| d.temp.as_ref(),
            None,
        ),
        net_svg: render_net_down_chart(days),
    }
}

fn render_raw_trio(points: &[RawPoint], prefix: &str) -> (String, String, String) {
    let cpu_pts: Vec<_> = points
        .iter()
        .map(|p| (p.timestamp, Some(p.cpu_pct), p.net_ok))
        .collect();
    let mem_pts: Vec<_> = points
        .iter()
        .map(|p| (p.timestamp, Some(p.mem_pct), p.net_ok))
        .collect();
    let temp_pts: Vec<_> = points
        .iter()
        .map(|p| (p.timestamp, p.temp_c, p.net_ok))
        .collect();

    (
        render_raw_line_chart(
            &format!("{prefix} CPU Load %"),
            "%",
            "--blue",
            &cpu_pts,
            Some((0.0, 100.0)),
        ),
        render_raw_line_chart(
            &format!("{prefix} Memory Usage %"),
            "%",
            "--mauve",
            &mem_pts,
            Some((0.0, 100.0)),
        ),
        render_raw_line_chart(
            &format!("{prefix} Temperature °C"),
            "°C",
            "--peach",
            &temp_pts,
            None,
        ),
    )
}

fn build_live_summary(
    raw_days: &[(Date, std::path::PathBuf)],
    disk_days: &[(Date, std::path::PathBuf)],
) -> io::Result<LiveSummary> {
    // Load up to the two most recent raw files so a 24h window works across midnight.
    let mut live_points = Vec::new();
    let start_idx = raw_days.len().saturating_sub(2);
    for (_, path) in &raw_days[start_idx..] {
        live_points.extend(parse_raw_log(path)?);
    }

    let latest = live_points.last();
    let latest_ts = latest.map_or(0, |p| p.timestamp);

    let win_12h: Vec<_> = live_points
        .iter()
        .filter(|p| p.timestamp >= latest_ts.saturating_sub(SECS_12H))
        .cloned()
        .collect();
    let win_24h: Vec<_> = live_points
        .iter()
        .filter(|p| p.timestamp >= latest_ts.saturating_sub(SECS_24H))
        .cloned()
        .collect();

    let (cpu_12h_svg, mem_12h_svg, temp_12h_svg) =
        render_raw_trio(&downsample_raw(&win_12h, MAX_RAW_SVG_POINTS), "12h");
    let (cpu_24h_svg, mem_24h_svg, temp_24h_svg) =
        render_raw_trio(&downsample_raw(&win_24h, MAX_RAW_SVG_POINTS), "24h");

    let disk_svg = if let Some((_, disk_path)) = disk_days.last() {
        let disk_pts = parse_disks_log(disk_path)?;
        let latest_disks = latest_per_mount(&disk_pts);
        let refs: Vec<(&str, f32)> = latest_disks
            .iter()
            .map(|(m, pct)| (m.as_str(), *pct))
            .collect();
        render_disk_bars(&refs)
    } else {
        render_disk_bars(&[])
    };

    Ok(LiveSummary {
        latest_cpu: latest.map(|p| p.cpu_pct),
        latest_mem: latest.map(|p| p.mem_pct),
        latest_temp: latest.and_then(|p| p.temp_c),
        latest_net_ok: latest.map(|p| p.net_ok),
        disk_svg,
        cpu_12h_svg,
        mem_12h_svg,
        temp_12h_svg,
        cpu_24h_svg,
        mem_24h_svg,
        temp_24h_svg,
    })
}

fn latest_per_mount(points: &[DiskPoint]) -> Vec<(String, f32)> {
    let mut map: BTreeMap<String, f32> = BTreeMap::new();
    for p in points {
        map.insert(p.mount.clone(), p.used_pct);
    }
    map.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
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
                "brother_cache_test_{label}_{}_{}",
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
    fn test_cache_builds_evicts_and_respects_force() -> io::Result<()> {
        let dir = TestTempDir::new("cache")?;
        let past_raw = dir.path().join("raw-2026-11-10.log");
        let today_raw = dir.path().join("raw-2026-11-19.log");
        fs::write(&past_raw, "1000 10.0 40.0 45.0 1 10000\n")?;
        fs::write(&today_raw, "2000 25.0 50.0 48.0 1 10000\n")?;

        let mut cache = DashboardCache::new();
        let html = cache.build_dashboard(dir.path(), date!(2026 - 11 - 19), false)?;
        assert!(html.contains("Retained Raw (1d)"));
        assert!(html.contains("25.0%"));
        assert_eq!(cache.retained_days.len(), 1);

        // Removing the retired past file should evict it from the cache on next build.
        fs::remove_file(&past_raw)?;
        let html2 = cache.build_dashboard(dir.path(), date!(2026 - 11 - 19), false)?;
        assert!(html2.contains("Retained Raw (0d)"));
        assert_eq!(cache.retained_days.len(), 0);

        // Force rebuild on empty history still succeeds cleanly.
        let html3 = cache.build_dashboard(dir.path(), date!(2026 - 11 - 19), true)?;
        assert!(html3.contains("<!doctype html>"));
        Ok(())
    }
}
