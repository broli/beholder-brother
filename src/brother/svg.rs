use crate::brother::logs::{HistoryDay, MetricStats, RawPoint};
use std::fmt::Write as _;

const CHART_W: f32 = 800.0;
const CHART_H: f32 = 220.0;
const PAD_LEFT: f32 = 52.0;
const PAD_RIGHT: f32 = 18.0;
const PAD_TOP: f32 = 34.0;
const PAD_BOTTOM: f32 = 28.0;
const PLOT_W: f32 = CHART_W - PAD_LEFT - PAD_RIGHT;
const PLOT_H: f32 = CHART_H - PAD_TOP - PAD_BOTTOM;

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
        PAD_LEFT + PLOT_W / 2.0
    } else {
        PAD_LEFT + (idx as f32 / (len - 1) as f32) * PLOT_W
    }
}

fn map_x_time(ts: u64, t_min: u64, t_max: u64) -> f32 {
    if t_max <= t_min {
        return PAD_LEFT + PLOT_W / 2.0;
    }
    let span = (t_max - t_min) as f32;
    let normalized = (ts.saturating_sub(t_min) as f32 / span).clamp(0.0, 1.0);
    PAD_LEFT + normalized * PLOT_W
}

fn map_y(val: f32, y_min: f32, y_max: f32) -> f32 {
    let span = (y_max - y_min).max(0.001);
    let norm = if val.is_finite() {
        ((val - y_min) / span).clamp(0.0, 1.0)
    } else {
        0.0
    };
    PAD_TOP + (1.0 - norm) * PLOT_H
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
    start_svg_card_sized(buf, title, CHART_H);
}

fn start_svg_card_sized(buf: &mut String, title: &str, card_h: f32) {
    let escaped = escape_xml(title);
    let _ = writeln!(
        buf,
        r#"<svg viewBox="0 0 {CHART_W} {card_h}" class="chart-svg" role="img" aria-label="{escaped}">"#
    );
    let _ = writeln!(buf, "  <title>{escaped}</title>");
    let _ = writeln!(
        buf,
        r#"  <rect x="0.5" y="0.5" width="{w}" height="{h}" rx="6" fill="var(--surface)" stroke="var(--border)" stroke-width="1"/>"#,
        w = CHART_W - 1.0,
        h = card_h - 1.0
    );
    let _ = writeln!(
        buf,
        r#"  <text x="16" y="22" font-size="13" font-weight="600" fill="var(--ink)">{escaped}</text>"#
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
            r#"  <text x="{x:.1}" y="{ty:.1}" font-size="10" text-anchor="end" fill="var(--muted)">{val:.0}{unit}</text>"#,
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
        r#"  <text x="{cx}" y="{cy}" font-size="13" text-anchor="middle" fill="var(--muted)">{}</text>"#,
        escape_xml(message)
    );
    buf.push_str("</svg>\n");
}

fn write_raw_segment(
    buf: &mut String,
    segment: &mut String,
    first: &mut Option<(f32, f32, f32)>,
    count: &mut usize,
    color_var: &str,
    unit: &str,
) {
    if *count == 1
        && let Some((x, y, val)) = first.take()
    {
        let _ = writeln!(
            buf,
            r#"  <circle cx="{x:.1}" cy="{y:.1}" r="3.5" fill="var({color_var})"><title>{val:.1}{unit}</title></circle>"#,
            unit = escape_xml(unit)
        );
    } else if *count > 1 {
        let _ = writeln!(
            buf,
            r#"  <polyline fill="none" stroke="var({color_var})" stroke-width="2" stroke-linejoin="round" points="{segment}"/>"#
        );
    }
    segment.clear();
    *first = None;
    *count = 0;
}

pub fn render_raw_line_chart(
    title: &str,
    unit: &str,
    color_var: &str,
    points: &[RawPoint],
    extract: fn(&RawPoint) -> Option<f32>,
    fixed_bounds: Option<(f32, f32)>,
    x_bounds: (u64, u64),
) -> String {
    let mut buf = String::with_capacity(2048);
    start_svg_card(&mut buf, title);
    let (t_min, t_max) = x_bounds;

    let has_values = points.iter().any(|p| extract(p).is_some());
    if points.is_empty() || !has_values {
        draw_empty_state(&mut buf, "No samples available");
        return buf;
    }

    let (y_min, y_max) = compute_bounds(points.iter().filter_map(extract), fixed_bounds);
    draw_y_grid(&mut buf, y_min, y_max, unit);

    let mut segment = String::new();
    let mut first = None;
    let mut count = 0usize;
    for p in points {
        if let Some(val) = extract(p) {
            let x = map_x_time(p.timestamp, t_min, t_max);
            let y = map_y(val, y_min, y_max);
            if count == 0 {
                first = Some((x, y, val));
            } else {
                segment.push(' ');
            }
            let _ = write!(segment, "{x:.1},{y:.1}");
            count += 1;
        } else {
            write_raw_segment(
                &mut buf,
                &mut segment,
                &mut first,
                &mut count,
                color_var,
                unit,
            );
        }
    }
    write_raw_segment(
        &mut buf,
        &mut segment,
        &mut first,
        &mut count,
        color_var,
        unit,
    );

    let base_y = PAD_TOP + PLOT_H;
    for p in points {
        if !p.net_ok {
            let x = map_x_time(p.timestamp, t_min, t_max);
            let _ = writeln!(
                buf,
                r#"  <line x1="{x:.1}" y1="{y1:.1}" x2="{x:.1}" y2="{base_y:.1}" stroke="var(--status-err)" stroke-width="2"><title>Network down @ {ts}</title></line>"#,
                ts = p.timestamp,
                y1 = base_y - 8.0
            );
        }
    }

    if let Some(last_v) = points.iter().rev().find_map(extract) {
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
        buf.push_str(r#"  <polygon fill="var("#);
        buf.push_str(color_var);
        buf.push_str(r#")" fill-opacity="0.18" points=""#);
        for (i, &(_, s)) in active.iter().enumerate() {
            let x = map_x(i, len);
            let y = map_y(s.max, y_min, y_max);
            let _ = write!(buf, "{x:.1},{y:.1} ");
        }
        for (i, &(_, s)) in active.iter().enumerate().rev() {
            let x = map_x(i, len);
            let y = map_y(s.min, y_min, y_max);
            if i == 0 {
                let _ = write!(buf, "{x:.1},{y:.1}");
            } else {
                let _ = write!(buf, "{x:.1},{y:.1} ");
            }
        }
        buf.push_str("\"/>\n");

        let _ = write!(
            buf,
            r#"  <polyline fill="none" stroke="var({color_var})" stroke-opacity="0.7" stroke-width="1.5" stroke-dasharray="4 3" points=""#
        );
        for (i, &(_, s)) in active.iter().enumerate() {
            let x = map_x(i, len);
            let y = map_y(s.p95, y_min, y_max);
            if i > 0 {
                buf.push(' ');
            }
            let _ = write!(buf, "{x:.1},{y:.1}");
        }
        buf.push_str("\"/>\n");

        let _ = write!(
            buf,
            r#"  <polyline fill="none" stroke="var({color_var})" stroke-width="2.2" stroke-linejoin="round" points=""#
        );
        for (i, &(_, s)) in active.iter().enumerate() {
            let x = map_x(i, len);
            let y = map_y(s.avg, y_min, y_max);
            if i > 0 {
                buf.push(' ');
            }
            let _ = write!(buf, "{x:.1},{y:.1}");
        }
        buf.push_str("\"/>\n");
    }

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

    if let (Some((first_d, _)), Some((last_d, _))) = (active.first(), active.last()) {
        let y_lbl = CHART_H - 8.0;
        let _ = writeln!(
            buf,
            r#"  <text x="{PAD_LEFT:.1}" y="{y_lbl:.1}" font-size="10" fill="var(--muted)">{first_d}</text>"#
        );
        if active.len() > 1 {
            let _ = writeln!(
                buf,
                r#"  <text x="{x:.1}" y="{y_lbl:.1}" font-size="10" text-anchor="end" fill="var(--muted)">{last_d}</text>"#,
                x = CHART_W - PAD_RIGHT
            );
        }
    }

    buf.push_str("</svg>\n");
    buf
}

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

    let slot_w = PLOT_W / active.len() as f32;
    let bar_w = (slot_w * 0.65).clamp(3.0, 28.0);
    let base_y = PAD_TOP + PLOT_H;

    for (i, &(date, sec)) in active.iter().enumerate() {
        let cx = PAD_LEFT + (i as f32 + 0.5) * slot_w;
        let x = cx - bar_w / 2.0;
        let top_y = map_y(sec as f32, 0.0, max_sec);
        let h = (base_y - top_y).max(2.0);
        let y = base_y - h;
        let fill = if sec == 0 {
            "var(--status-ok)"
        } else {
            "var(--status-err)"
        };
        let _ = writeln!(
            buf,
            r#"  <rect x="{x:.1}" y="{y:.1}" width="{bar_w:.1}" height="{h:.1}" rx="2" fill="{fill}"><title>{date}: {sec}s down</title></rect>"#
        );
    }

    buf.push_str("</svg>\n");
    buf
}

pub fn render_disk_bars(mounts: &[(&str, f32)]) -> String {
    let mut buf = String::with_capacity(1024);
    if mounts.is_empty() {
        start_svg_card(&mut buf, "Disk Usage (latest)");
        draw_empty_state(&mut buf, "No disk samples available");
        return buf;
    }

    let row_h = if mounts.len() <= 5 {
        (PLOT_H / mounts.len() as f32).clamp(22.0, 34.0)
    } else {
        28.0
    };
    let card_h = (PAD_TOP + 12.0 + mounts.len() as f32 * row_h).max(CHART_H);
    start_svg_card_sized(&mut buf, "Disk Usage (latest)", card_h);

    let bar_x = 140.0;
    let bar_max_w = CHART_W - bar_x - 70.0;

    for (i, &(mount, pct)) in mounts.iter().enumerate() {
        let y = PAD_TOP + 6.0 + i as f32 * row_h;
        let clamped = if pct.is_finite() {
            pct.clamp(0.0, 100.0)
        } else {
            0.0
        };
        let fill_w = (clamped / 100.0) * bar_max_w;
        let color = if clamped >= 90.0 {
            "var(--status-err)"
        } else if clamped >= 75.0 {
            "var(--status-warn)"
        } else {
            "var(--status-ok)"
        };

        let _ = writeln!(
            buf,
            r#"  <text x="{x:.1}" y="{ty:.1}" font-size="12" text-anchor="end" fill="var(--ink)">{}</text>"#,
            escape_xml(mount),
            x = bar_x - 10.0,
            ty = y + 13.0
        );
        let _ = writeln!(
            buf,
            r#"  <rect x="{bar_x}" y="{y:.1}" width="{bar_max_w:.1}" height="16" rx="3" fill="var(--grid)"/>"#
        );
        let _ = writeln!(
            buf,
            r#"  <rect x="{bar_x}" y="{y:.1}" width="{fill_w:.1}" height="16" rx="3" fill="{color}"/>"#
        );
        let _ = writeln!(
            buf,
            r#"  <text x="{lx:.1}" y="{ty:.1}" font-size="12" font-weight="600" fill="var(--ink)">{clamped:.1}%</text>"#,
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

    fn pt(timestamp: u64, cpu_pct: f32, temp_c: Option<f32>, net_ok: bool) -> RawPoint {
        RawPoint {
            timestamp,
            cpu_pct,
            mem_pct: 50.0,
            temp_c,
            net_ok,
            interval_ms: 10_000,
        }
    }

    #[test]
    fn test_svg_handles_empty_and_single_point_without_nan() {
        let empty = render_raw_line_chart(
            "CPU %",
            "%",
            "--series-cpu",
            &[],
            |p| Some(p.cpu_pct),
            Some((0.0, 100.0)),
            (0, 100),
        );
        assert!(empty.contains("viewBox=\"0 0 800 220\""));
        assert!(empty.contains("No samples available"));
        assert!(!empty.contains("NaN"));

        let single = render_raw_line_chart(
            "Temp",
            "°C",
            "--series-temp",
            &[pt(1000, 10.0, Some(48.5), false)],
            |p| p.temp_c,
            None,
            (100, 100),
        );
        assert!(single.contains("<circle"));
        assert!(single.contains("Network down"));
        assert!(!single.contains("NaN"));
    }

    #[test]
    fn test_svg_splits_none_gaps_and_escapes_xml() {
        let pts = vec![
            pt(1000, 10.0, Some(45.0), true),
            pt(1010, 10.0, Some(46.0), true),
            pt(1020, 10.0, None, true),
            pt(1030, 10.0, Some(47.0), true),
        ];
        let svg = render_raw_line_chart(
            "Temp <SoC>",
            "°C",
            "--series-temp",
            &pts,
            |p| p.temp_c,
            None,
            (100, 400),
        );
        assert!(svg.contains("Temp &lt;SoC&gt;"));
        assert_eq!(svg.matches("<polyline").count(), 1);
        assert_eq!(svg.matches("<circle").count(), 1);
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
            "--series-cpu",
            &days,
            |d| d.cpu.as_ref(),
            Some((0.0, 100.0)),
        );
        assert!(cpu_svg.contains("<polygon"));
        assert!(cpu_svg.contains("2026-11-10"));
        assert!(!cpu_svg.contains("NaN"));

        let net_svg = render_net_down_chart(&days);
        assert_eq!(net_svg.matches("<rect").count(), 3); // 1 card rect + 2 bars

        let disk_svg = render_disk_bars(&[("/", 62.5), ("/home", 81.0)]);
        assert!(disk_svg.contains("62.5%"));
        assert!(disk_svg.contains("81.0%"));

        let many_mounts = [
            ("/", 10.0),
            ("/home", 20.0),
            ("/var", 30.0),
            ("/tmp", 40.0),
            ("/mnt/a", 50.0),
            ("/mnt/b", 60.0),
            ("/mnt/c", 70.0),
        ];
        let tall_disk_svg = render_disk_bars(&many_mounts);
        assert!(tall_disk_svg.contains("/mnt/c"));
        assert!(tall_disk_svg.contains("70.0%"));
    }
}
