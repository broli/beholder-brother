use crate::Result;
use crate::brother::html::{
    self, DashboardData, HistoryView, LiveSummary, MetricChartSet, RetainedDayView,
};
use crate::brother::logs::{
    FileStamp, RawPoint, downsample_raw, parse_history, parse_latest_disks, parse_raw_log,
    scan_log_dir,
};
use crate::brother::svg::{
    render_disk_bars, render_history_band_chart, render_net_down_chart, render_raw_line_chart,
};
use std::collections::{BTreeMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use time::Date;

const MAX_RAW_SVG_POINTS: usize = 600;
const SECS_12H: u64 = 12 * 3600;
const SECS_24H: u64 = 24 * 3600;

struct CachedHistory {
    stamp: FileStamp,
    view: Arc<HistoryView>,
}

struct CachedDay {
    stamp: FileStamp,
    view: Arc<RetainedDayView>,
}

#[derive(PartialEq, Eq)]
struct PageCacheKey {
    today: Date,
    scale_bits: u32,
    history_stamp: Option<FileStamp>,
    live_raw_stamps: Vec<(Date, Option<FileStamp>)>,
    latest_disk_stamp: Option<(Date, Option<FileStamp>)>,
    retained_stamps: Vec<(Date, Option<FileStamp>)>,
}

impl PageCacheKey {
    fn matches(
        &self,
        today: Date,
        scale_bits: u32,
        history_stamp: Option<FileStamp>,
        live_raw_stamps: &[(Date, Option<FileStamp>)],
        latest_disk_stamp: Option<(Date, Option<FileStamp>)>,
        retained_files: &[(Date, &Path, Option<FileStamp>)],
    ) -> bool {
        self.today == today
            && self.scale_bits == scale_bits
            && self.history_stamp == history_stamp
            && self.latest_disk_stamp == latest_disk_stamp
            && self.live_raw_stamps == live_raw_stamps
            && self
                .retained_stamps
                .iter()
                .copied()
                .eq(retained_files.iter().map(|(d, _, s)| (*d, *s)))
    }
}

struct CachedPage {
    key: PageCacheKey,
    html: Arc<str>,
}

#[derive(Default)]
pub struct DashboardCache {
    history: Option<CachedHistory>,
    retained_days: BTreeMap<Date, CachedDay>,
    last_page: Option<CachedPage>,
}

impl DashboardCache {
    pub fn build_dashboard(
        &mut self,
        log_dir: &Path,
        today: Date,
        force: bool,
        scale: f32,
    ) -> Result<Arc<str>> {
        if force {
            self.history = None;
            self.retained_days.clear();
            self.last_page = None;
        }

        let inventory = scan_log_dir(log_dir)?;

        let history_stamp = inventory
            .history_path
            .as_deref()
            .map(FileStamp::from_path)
            .transpose()?
            .flatten();

        let live_raw_start = inventory.raw_days.len().saturating_sub(2);
        let live_raw_stamps = inventory.raw_days[live_raw_start..]
            .iter()
            .map(|(date, path)| FileStamp::from_path(path).map(|s| (*date, s)))
            .collect::<Result<Vec<_>>>()?;

        let latest_disk_stamp = inventory
            .disk_days
            .last()
            .map(|(date, path)| FileStamp::from_path(path).map(|s| (*date, s)))
            .transpose()?;

        let mut retained_files = Vec::new();
        for (date, path) in &inventory.raw_days {
            if *date < today {
                retained_files.push((*date, path.as_path(), FileStamp::from_path(path)?));
            }
        }

        let scale_bits = scale.to_bits();
        if !force
            && let Some(cached) = &self.last_page
            && cached.key.matches(
                today,
                scale_bits,
                history_stamp,
                &live_raw_stamps,
                latest_disk_stamp,
                &retained_files,
            )
        {
            return Ok(Arc::clone(&cached.html));
        }

        let retained_stamps =
            (!force).then(|| retained_files.iter().map(|(d, _, s)| (*d, *s)).collect());

        let history = self.load_history(inventory.history_path.as_deref(), history_stamp)?;

        let active_dates: HashSet<Date> = inventory.raw_days.iter().map(|(d, _)| *d).collect();
        self.retained_days.retain(|d, _| active_dates.contains(d));

        let mut retained = Vec::with_capacity(retained_files.len());
        for (date, path, stamp) in retained_files {
            retained.push(self.load_retained_day(date, path, stamp)?);
        }

        let live = build_live_summary(&inventory.raw_days[live_raw_start..], &inventory.disk_days)?;
        let log_dir_display = log_dir.display().to_string();

        let data = DashboardData {
            log_dir_display: &log_dir_display,
            scale,
            live: &live,
            retained: &retained,
            history: &history,
        };
        let html: Arc<str> = Arc::from(html::render_page(&data));
        if let Some(retained_stamps) = retained_stamps {
            self.last_page = Some(CachedPage {
                key: PageCacheKey {
                    today,
                    scale_bits,
                    history_stamp,
                    live_raw_stamps,
                    latest_disk_stamp,
                    retained_stamps,
                },
                html: Arc::clone(&html),
            });
        }
        Ok(html)
    }

    fn load_history(
        &mut self,
        history_path: Option<&Path>,
        stamp: Option<FileStamp>,
    ) -> Result<Arc<HistoryView>> {
        let (Some(path), Some(stamp)) = (history_path, stamp) else {
            self.history = None;
            return Ok(Arc::new(render_history_view(&[])));
        };

        if let Some(cached) = &self.history
            && cached.stamp == stamp
        {
            return Ok(Arc::clone(&cached.view));
        }

        let days = parse_history(path)?;
        let view = Arc::new(render_history_view(&days));
        self.history = Some(CachedHistory {
            stamp,
            view: Arc::clone(&view),
        });
        Ok(view)
    }

    fn load_retained_day(
        &mut self,
        date: Date,
        path: &Path,
        stamp: Option<FileStamp>,
    ) -> Result<Arc<RetainedDayView>> {
        let stamp = stamp.unwrap_or(FileStamp {
            modified: None,
            len: 0,
        });

        if let Some(cached) = self.retained_days.get(&date)
            && cached.stamp == stamp
        {
            return Ok(Arc::clone(&cached.view));
        }

        let raw = parse_raw_log(path)?;
        let samples = raw.len();
        let outages = raw.iter().filter(|p| !p.net_ok).count();
        let downsampled = downsample_raw(&raw, MAX_RAW_SVG_POINTS);
        let charts = render_raw_trio(&downsampled, "Daily");

        let view = Arc::new(RetainedDayView {
            date_label: date.to_string(),
            samples,
            outages,
            charts,
        });
        self.retained_days.insert(
            date,
            CachedDay {
                stamp,
                view: Arc::clone(&view),
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
            "--series-cpu",
            days,
            |d| d.cpu.as_ref(),
            Some((0.0, 100.0)),
        ),
        mem_svg: render_history_band_chart(
            "Memory Usage History (%)",
            "%",
            "--series-mem",
            days,
            |d| d.mem.as_ref(),
            Some((0.0, 100.0)),
        ),
        temp_svg: render_history_band_chart(
            "Temperature History (°C)",
            "°C",
            "--series-temp",
            days,
            |d| d.temp.as_ref(),
            None,
        ),
        net_svg: render_net_down_chart(days),
    }
}

fn render_raw_trio(points: &[RawPoint], prefix: &str) -> MetricChartSet {
    MetricChartSet {
        cpu_svg: render_raw_line_chart(
            &format!("{prefix} CPU Load %"),
            "%",
            "--series-cpu",
            points,
            |p| Some(p.cpu_pct),
            Some((0.0, 100.0)),
        ),
        mem_svg: render_raw_line_chart(
            &format!("{prefix} Memory Usage %"),
            "%",
            "--series-mem",
            points,
            |p| Some(p.mem_pct),
            Some((0.0, 100.0)),
        ),
        temp_svg: render_raw_line_chart(
            &format!("{prefix} Temperature °C"),
            "°C",
            "--series-temp",
            points,
            |p| p.temp_c,
            None,
        ),
    }
}

fn build_live_summary(
    recent_raw_days: &[(Date, std::path::PathBuf)],
    disk_days: &[(Date, std::path::PathBuf)],
) -> Result<LiveSummary> {
    let mut live_points = Vec::new();
    for (_, path) in recent_raw_days {
        live_points.extend(parse_raw_log(path)?);
    }

    let latest = live_points.last();
    let latest_ts = latest.map_or(0, |p| p.timestamp);

    let idx_12h = live_points.partition_point(|p| p.timestamp < latest_ts.saturating_sub(SECS_12H));
    let idx_24h = live_points.partition_point(|p| p.timestamp < latest_ts.saturating_sub(SECS_24H));

    let window_12h = render_raw_trio(
        &downsample_raw(&live_points[idx_12h..], MAX_RAW_SVG_POINTS),
        "12h",
    );
    let window_24h = render_raw_trio(
        &downsample_raw(&live_points[idx_24h..], MAX_RAW_SVG_POINTS),
        "24h",
    );

    let disk_svg = if let Some((_, disk_path)) = disk_days.last() {
        let latest_mounts = parse_latest_disks(disk_path)?;
        let refs: Vec<(&str, f32)> = latest_mounts
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
        window_12h,
        window_24h,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestTempDir;
    use std::fs;
    use time::macros::date;

    #[test]
    fn test_cache_builds_evicts_and_respects_force() -> Result<()> {
        let dir = TestTempDir::new("cache")?;
        let past_raw = dir.path().join("raw-2026-11-10.log");
        let today_raw = dir.path().join("raw-2026-11-19.log");
        fs::write(&past_raw, "1000 10.0 40.0 45.0 1 10000\n")?;
        fs::write(&today_raw, "2000 25.0 50.0 48.0 1 10000\n")?;

        let mut cache = DashboardCache::default();
        let html = cache.build_dashboard(dir.path(), date!(2026 - 11 - 19), false, 1.0)?;
        assert!(html.contains("Retained Raw (1d)"));
        assert!(html.contains("25.0%"));
        assert_eq!(cache.retained_days.len(), 1);
        assert!(cache.last_page.is_some());

        let html_cached = cache.build_dashboard(dir.path(), date!(2026 - 11 - 19), false, 1.0)?;
        assert_eq!(html, html_cached);

        // Removing the retired past file should evict it from the cache on next build.
        fs::remove_file(&past_raw)?;
        let html2 = cache.build_dashboard(dir.path(), date!(2026 - 11 - 19), false, 1.0)?;
        assert!(html2.contains("Retained Raw (0d)"));
        assert_eq!(cache.retained_days.len(), 0);

        let html3 = cache.build_dashboard(dir.path(), date!(2026 - 11 - 19), true, 1.5)?;
        assert!(html3.contains("<!DOCTYPE html>"));
        assert!(html3.contains("--scale: 1.50"));
        assert!(cache.last_page.is_none());
        Ok(())
    }
}
