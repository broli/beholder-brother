use crate::brother::logs::{HistoryDay, MetricStats};
use std::fmt::Write as _;

const CHART_W: f32 = 800.0;
const CHART_H: f32 = 220.0;
const SHADOW_OFFSET: f32 = 5.0;
const PAD_LEFT: f32 = 52.0;
const PAD_RIGHT: f32 = 18.0;
const PAD_TOP: f32 = 34.0;
const PAD_BOTTOM: f32 = 28.0;

fn plot_w() -> f32 {
    CHART_W - PAD_LEFT - PAD_RIGHT
}

fn plot_h() -> f32 {
    CHART_H - PAD_TOP - PAD_BOTTOM
}

/// Escapes XML/HTML special characters so dynamic labels (e.g. mount paths) are XSS-safe.
pub fn escape_xml(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

fn map_x(idx: usize, len: usize) -> f32 {
    if len <= 1 {
        PAD_LEFT + plot_w() / 2.0
    } else {
        PAD_LEFT + (idx as f32 / (len - 1) as f32) * plot_w()
    }
}

fn map_y(val: f32, y_min: f32, y_max: f32) -> f32 {
    let span = (y_max - y_min).max(0.001);
    let norm = ((val - y_min) / span).clamp(0.0, 1.0);
    PAD_TOP + (1.0 - norm) * plot_h()
}

fn compute_bounds(values: impl Iterator<Item = f32>, fixed: Option<(f32, f32)>) -> (f32, f32) {
    if let Some((min, max)) = fixed {
        return (min, max);
    }
    let mut min_v = f32::INFINITY;
    let mut max_v = f32::NEG_INFINITY;
    for v in values {
        if v.is_finite() {
            min_v = min_v.min(v);
            max_v = max_v.max(v);
        }
    }
    if !min_v.is_finite() || !max_v.is_finite() {
        return (0.0, 100.0);
    }
    if (max_v - min_v).abs() < 1.0 {
        (min_v - 5.0, max_v + 5.0)
    } else {
        let pad = (max_v - min_v) * 0.1;
        (min_v - pad, max_v + pad)
    }
}

fn start_svg_card(buf: &mut String, title: &str) {
    let total_w = CHART_W + SHADOW_OFFSET;
    let total_h = CHART_H + SHADOW_OFFSET;
    let _ = writeln!(
        buf,
        r#"<svg viewBox="0 0 {total_w} {total_h}" class="chart-svg" role="img" aria-label="{title}">"#,
        title = escape_xml(title)
    );
    // Solid +5,+5 offset shadow card behind the chart frame
    let _ = writeln!(
        buf,
        r#"  <rect x="{SHADOW_OFFSET}" y="{SHADOW_OFFSET}" width="{CHART_W}" height="{CHART_H}" rx="10" fill="var(--shadow)"/>"#
    );
    let _ = writeln!(
        buf,
        r#"  <rect x="0" y="0" width="{CHART_W}" height="{CHART_H}" rx="10" fill="var(--surface)" stroke="var(--overlay)" stroke-width="1"/>"#
    );
    let _ = writeln!(
        buf,
        r#"  <text x="16" y="22" font-size="13" font-weight="600" fill="var(--text)">{}</text>"#,
        escape_xml(title)
    );
}

fn draw_y_grid(buf: &mut String, y_min: f32, y_max: f32, unit: &str) {
    for step in 0..=4 {
        let frac = step as f32 / 4.0;
        let val = y_min + frac * (y_max - y_min);
        let y = map_y(val, y_min, y_max);
        let _ = writeln!(
            buf,
            r#"  <line x1="{PAD_LEFT}" y1="{y:.1}" x2="{x2:.1}" y2="{y:.1}" stroke="var(--grid)" stroke-width="1"/>"#,
            x2 = CHART_W - PAD_RIGHT
        );
        let _ = writeln!(
            buf,
            r#"  <text x="{x:.1}" y="{ty:.1}" font-size="10" text-anchor="end" fill="var(--subtext)">{val:.0}{unit}</text>"#,
            x = PAD_LEFT - 6.0,
            ty = y + 3.5,
            unit = escape_xml(unit)
        );
    }
}

fn draw_empty_state(buf: &mut String, message: &str) {
    let cx = CHART_W / 2.0;
    let cy = CHART_H / 2.0 + 6.0;
    let _ = writeln!(
        buf,
        r#"  <text x="{cx}" y="{cy}" font-size="13" text-anchor="middle" fill="var(--subtext)">{}</text>"#,
        escape_xml(message)
    );
    buf.push_str("</svg>\n");
}

/// Renders an intraday line chart from `(timestamp, Option<value>, net_ok)` tuples.
pub fn render_raw_line_chart(
    title: &str,
    unit: &str,
    color_var: &str,
    points: &[(u64, Option<f32>, bool)],
    fixed_bounds: Option<(f32, f32)>,
) -> String {
    let mut buf = String::with_capacity(2048);
    start_svg_card(&mut buf, title);

    let has_values = points.iter().any(|(_, v, _)| v.is_some());
    if points.is_empty() || !has_values {
        draw_empty_state(&mut buf, "No samples available");
        return buf;
    }

    let (y_min, y_max) = compute_bounds(points.iter().filter_map(|(_, v, _)| *v), fixed_bounds);
    draw_y_grid(&mut buf, y_min, y_max, unit);

    let flush_segment = |buf: &mut String, segment: &mut String| {
        if !segment.is_empty() {
            let _ = writeln!(
                buf,
                r#"  <polyline fill="none" stroke="var({color_var})" stroke-width="2" stroke-linejoin="round" points="{segment}"/>"#
            );
            segment.clear();
        }
    };

    // Draw contiguous polyline segments (splitting on None gaps such as missing temp_c)
    let mut segment = String::new();
    for (i, &(_, val_opt, _)) in points.iter().enumerate() {
        if let Some(val) = val_opt {
            let x = map_x(i, points.len());
            let y = map_y(val, y_min, y_max);
            if !segment.is_empty() {
                segment.push(' ');
            }
            let _ = write!(segment, "{x:.1},{y:.1}");
        } else {
            flush_segment(&mut buf, &mut segment);
        }
    }
    flush_segment(&mut buf, &mut segment);

    // Highlight single points or network outage ticks at the bottom axis
    let base_y = PAD_TOP + plot_h();
    for (i, &(ts, val_opt, net_ok)) in points.iter().enumerate() {
        let x = map_x(i, points.len());
        if !net_ok {
            let _ = writeln!(
                buf,
                r#"  <line x1="{x:.1}" y1="{y1:.1}" x2="{x:.1}" y2="{base_y:.1}" stroke="var(--red)" stroke-width="2"><title>Network down @ {ts}</title></line>"#,
                y1 = base_y - 8.0
            );
        }
        if points.len() == 1
            && let Some(val) = val_opt
        {
            let y = map_y(val, y_min, y_max);
            let _ = writeln!(
                buf,
                r#"  <circle cx="{x:.1}" cy="{y:.1}" r="3.5" fill="var({color_var})"><title>{val:.1}{unit}</title></circle>"#,
                unit = escape_xml(unit)
            );
        }
    }

    // Latest value badge in top-right corner
    if let Some((_, Some(last_v), _)) = points.iter().rev().find(|(_, v, _)| v.is_some()) {
        let _ = writeln!(
            buf,
            r#"  <text x="{x:.1}" y="22" font-size="12" font-weight="600" text-anchor="end" fill="var({color_var})">latest: {last_v:.1}{unit}</text>"#,
            x = CHART_W - PAD_RIGHT,
            unit = escape_xml(unit)
        );
    }

    buf.push_str("</svg>\n");
    buf
}

/// Renders a multi-day `history.log` chart with min-max shaded band, dashed p95, and solid avg.
pub fn render_history_band_chart(
    title: &str,
    unit: &str,
    color_var: &str,
    days: &[HistoryDay],
    extract: fn(&HistoryDay) -> Option<&MetricStats>,
    fixed_bounds: Option<(f32, f32)>,
) -> String {
    let mut buf = String::with_capacity(2048);
    start_svg_card(&mut buf, title);

    let active: Vec<_> = days
        .iter()
        .filter_map(|d| extract(d).map(|s| (d.date, s)))
        .collect();
    if active.is_empty() {
        draw_empty_state(&mut buf, "No historical data yet");
        return buf;
    }

    let (y_min, y_max) = compute_bounds(
        active.iter().flat_map(|(_, s)| [s.min, s.max]),
        fixed_bounds,
    );
    draw_y_grid(&mut buf, y_min, y_max, unit);

    let len = active.len();
    if len >= 2 {
        // Min-max shaded polygon: top edge (max) left-to-right, bottom edge (min) right-to-left
        let mut poly = String::new();
        for (i, &(_, s)) in active.iter().enumerate() {
            let x = map_x(i, len);
            let y = map_y(s.max, y_min, y_max);
            let _ = write!(poly, "{x:.1},{y:.1} ");
        }
        for (i, &(_, s)) in active.iter().enumerate().rev() {
            let x = map_x(i, len);
            let y = map_y(s.min, y_min, y_max);
            let _ = write!(poly, "{x:.1},{y:.1} ");
        }
        let _ = writeln!(
            buf,
            r#"  <polygon fill="var({color_var})" fill-opacity="0.18" points="{}"/>"#,
            poly.trim_end()
        );

        let mut p95_pts = String::new();
        let mut avg_pts = String::new();
        for (i, &(_, s)) in active.iter().enumerate() {
            let x = map_x(i, len);
            let yp = map_y(s.p95, y_min, y_max);
            let ya = map_y(s.avg, y_min, y_max);
            let _ = write!(p95_pts, "{x:.1},{yp:.1} ");
            let _ = write!(avg_pts, "{x:.1},{ya:.1} ");
        }
        let _ = writeln!(
            buf,
            r#"  <polyline fill="none" stroke="var({color_var})" stroke-opacity="0.7" stroke-width="1.5" stroke-dasharray="4 3" points="{}"/>"#,
            p95_pts.trim_end()
        );
        let _ = writeln!(
            buf,
            r#"  <polyline fill="none" stroke="var({color_var})" stroke-width="2.2" stroke-linejoin="round" points="{}"/>"#,
            avg_pts.trim_end()
        );
    }

    // Interactive hover dots with native <title> tooltips
    for (i, &(date, s)) in active.iter().enumerate() {
        let x = map_x(i, len);
        let y = map_y(s.avg, y_min, y_max);
        let _ = writeln!(
            buf,
            r#"  <circle cx="{x:.1}" cy="{y:.1}" r="3" fill="var({color_var})"><title>{date}: avg {avg:.1}{u} | p95 {p95:.1}{u} | min {min:.1}{u} | max {max:.1}{u}</title></circle>"#,
            avg = s.avg,
            p95 = s.p95,
            min = s.min,
            max = s.max,
            u = escape_xml(unit)
        );
    }

    // First and last date labels on X axis
    if let (Some((first_d, _)), Some((last_d, _))) = (active.first(), active.last()) {
        let y_lbl = CHART_H - 8.0;
        let _ = writeln!(
            buf,
            r#"  <text x="{PAD_LEFT:.1}" y="{y_lbl:.1}" font-size="10" fill="var(--subtext)">{first_d}</text>"#
        );
        if active.len() > 1 {
            let _ = writeln!(
                buf,
                r#"  <text x="{x:.1}" y="{y_lbl:.1}" font-size="10" text-anchor="end" fill="var(--subtext)">{last_d}</text>"#,
                x = CHART_W - PAD_RIGHT
            );
        }
    }

    buf.push_str("</svg>\n");
    buf
}

/// Renders a daily bar chart for `net_down_sec` from `history.log`.
pub fn render_net_down_chart(days: &[HistoryDay]) -> String {
    let mut buf = String::with_capacity(1536);
    start_svg_card(&mut buf, "Network Downtime (seconds/day)");

    let active: Vec<_> = days
        .iter()
        .filter_map(|d| d.net_down_sec.map(|sec| (d.date, sec)))
        .collect();
    if active.is_empty() {
        draw_empty_state(&mut buf, "No network history yet");
        return buf;
    }

    let max_sec = active
        .iter()
        .map(|(_, s)| *s as f32)
        .fold(10.0_f32, f32::max);
    draw_y_grid(&mut buf, 0.0, max_sec, "s");

    let slot_w = plot_w() / active.len() as f32;
    let bar_w = (slot_w * 0.65).clamp(3.0, 28.0);
    let base_y = PAD_TOP + plot_h();

    for (i, &(date, sec)) in active.iter().enumerate() {
        let cx = PAD_LEFT + (i as f32 + 0.5) * slot_w;
        let x = cx - bar_w / 2.0;
        let top_y = map_y(sec as f32, 0.0, max_sec);
        let h = (base_y - top_y).max(2.0);
        let y = base_y - h;
        let fill = if sec == 0 {
            "var(--green)"
        } else {
            "var(--red)"
        };
        let _ = writeln!(
            buf,
            r#"  <rect x="{x:.1}" y="{y:.1}" width="{bar_w:.1}" height="{h:.1}" rx="2" fill="{fill}"><title>{date}: {sec}s down</title></rect>"#
        );
    }

    buf.push_str("</svg>\n");
    buf
}

/// Renders horizontal disk usage bars for the latest snapshot of each mount point.
pub fn render_disk_bars(mounts: &[(&str, f32)]) -> String {
    let mut buf = String::with_capacity(1024);
    start_svg_card(&mut buf, "Disk Usage (latest)");

    if mounts.is_empty() {
        draw_empty_state(&mut buf, "No disk samples available");
        return buf;
    }

    let bar_x = 140.0;
    let bar_max_w = CHART_W - bar_x - 70.0;
    let row_h = (plot_h() / mounts.len().max(1) as f32).clamp(22.0, 34.0);

    for (i, &(mount, pct)) in mounts.iter().take(5).enumerate() {
        let y = PAD_TOP + 6.0 + i as f32 * row_h;
        let clamped = pct.clamp(0.0, 100.0);
        let fill_w = (clamped / 100.0) * bar_max_w;
        let color = if clamped >= 90.0 {
            "var(--red)"
        } else if clamped >= 75.0 {
            "var(--peach)"
        } else {
            "var(--green)"
        };

        let _ = writeln!(
            buf,
            r#"  <text x="{x:.1}" y="{ty:.1}" font-size="12" text-anchor="end" fill="var(--text)">{}</text>"#,
            escape_xml(mount),
            x = bar_x - 10.0,
            ty = y + 13.0
        );
        let _ = writeln!(
            buf,
            r#"  <rect x="{bar_x}" y="{y:.1}" width="{bar_max_w:.1}" height="16" rx="4" fill="var(--grid)"/>"#
        );
        let _ = writeln!(
            buf,
            r#"  <rect x="{bar_x}" y="{y:.1}" width="{fill_w:.1}" height="16" rx="4" fill="{color}"/>"#
        );
        let _ = writeln!(
            buf,
            r#"  <text x="{lx:.1}" y="{ty:.1}" font-size="12" font-weight="600" fill="var(--text)">{clamped:.1}%</text>"#,
            lx = bar_x + bar_max_w + 8.0,
            ty = y + 13.0
        );
    }

    buf.push_str("</svg>\n");
    buf
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::date;

    #[test]
    fn test_svg_handles_empty_and_single_point_without_nan() {
        let empty = render_raw_line_chart("CPU %", "%", "--blue", &[], Some((0.0, 100.0)));
        assert!(empty.contains("viewBox=\"0 0 805 225\""));
        assert!(empty.contains("No samples available"));
        assert!(!empty.contains("NaN"));

        let single =
            render_raw_line_chart("Temp", "°C", "--peach", &[(1000, Some(48.5), false)], None);
        assert!(single.contains("<circle"));
        assert!(single.contains("Network down"));
        assert!(!single.contains("NaN"));
    }

    #[test]
    fn test_svg_splits_none_gaps_and_escapes_xml() {
        let pts = vec![
            (1000, Some(45.0), true),
            (1010, None, true),
            (1020, Some(47.0), true),
        ];
        let svg = render_raw_line_chart("Temp <SoC>", "°C", "--peach", &pts, None);
        assert!(svg.contains("Temp &lt;SoC&gt;"));
        assert_eq!(svg.matches("<polyline").count(), 2);
    }

    #[test]
    fn test_history_band_and_net_and_disk_svgs() {
        let days = vec![
            HistoryDay {
                date: date!(2026 - 11 - 10),
                cpu: Some(MetricStats {
                    min: 1.0,
                    avg: 5.0,
                    max: 20.0,
                    p95: 15.0,
                }),
                mem: None,
                temp: None,
                net_down_sec: Some(0),
            },
            HistoryDay {
                date: date!(2026 - 11 - 11),
                cpu: Some(MetricStats {
                    min: 2.0,
                    avg: 8.0,
                    max: 30.0,
                    p95: 22.0,
                }),
                mem: None,
                temp: None,
                net_down_sec: Some(30),
            },
        ];

        let cpu_svg = render_history_band_chart(
            "CPU History",
            "%",
            "--blue",
            &days,
            |d| d.cpu.as_ref(),
            Some((0.0, 100.0)),
        );
        assert!(cpu_svg.contains("<polygon"));
        assert!(cpu_svg.contains("2026-11-10"));
        assert!(!cpu_svg.contains("NaN"));

        let net_svg = render_net_down_chart(&days);
        assert_eq!(net_svg.matches("<rect").count(), 4); // 2 card rects + 2 bars

        let disk_svg = render_disk_bars(&[("/", 62.5), ("/home", 81.0)]);
        assert!(disk_svg.contains("62.5%"));
        assert!(disk_svg.contains("81.0%"));
    }
}
