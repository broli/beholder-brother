use crate::brother::svg::escape_xml;
use std::fmt::{self, Write as _};
use std::sync::Arc;

const CSS: &str = include_str!("dashboard.css");
const JS: &str = include_str!("dashboard.js");

pub struct MetricChartSet {
    pub cpu_svg: String,
    pub mem_svg: String,
    pub temp_svg: String,
}

pub struct LiveSummary {
    pub latest_cpu: Option<f32>,
    pub latest_mem: Option<f32>,
    pub latest_temp: Option<f32>,
    pub latest_net_ok: Option<bool>,
    pub disk_svg: String,
    pub window_12h: MetricChartSet,
    pub window_24h: MetricChartSet,
}

pub struct RetainedDayView {
    pub date_label: String,
    pub samples: usize,
    pub outages: usize,
    pub charts: MetricChartSet,
}

pub struct HistoryView {
    pub days_count: usize,
    pub cpu_svg: String,
    pub mem_svg: String,
    pub temp_svg: String,
    pub net_svg: String,
}

pub struct DashboardData<'a> {
    pub log_dir_display: &'a str,
    pub scale: f32,
    pub live: &'a LiveSummary,
    pub retained: &'a [Arc<RetainedDayView>],
    pub history: &'a HistoryView,
}

struct OptMetric(Option<f32>, &'static str);

impl fmt::Display for OptMetric {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Some(v) => write!(f, "{v:.1}{}", self.1),
            None => f.write_str("-"),
        }
    }
}

fn write_chart_trio(html: &mut String, charts: &MetricChartSet) {
    html.push_str(&charts.cpu_svg);
    html.push_str(&charts.mem_svg);
    html.push_str(&charts.temp_svg);
}

pub fn render_page(data: &DashboardData<'_>) -> String {
    let mut html = String::with_capacity(32 * 1024 + data.retained.len() * 8 * 1024);
    let scale = data.scale;
    let net_badge = match data.live.latest_net_ok {
        Some(true) => r#"<span class="status-badge" data-status="ok">&#9679; ONLINE</span>"#,
        Some(false) => r#"<span class="status-badge" data-status="down">&#9650; DOWN</span>"#,
        None => "-",
    };

    let _ = write!(
        html,
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<meta name="color-scheme" content="light dark">
<title>brother | beholder telemetry</title>
<style>{CSS}
:root {{ --scale: {scale:.2}; }}
</style>
</head>
<body>
<div class="wrap">
<header class="page-header">
  <h1>brother</h1>
  <div class="meta">log-dir: {dir}</div>
</header>

<main>
<div class="tablist" role="tablist" aria-label="Dashboard views">
  <button type="button" role="tab" id="tab-btn-live" data-panel="panel-live" aria-selected="true" tabindex="0">Live (Now)</button>
  <button type="button" role="tab" id="tab-btn-retained" data-panel="panel-retained" aria-selected="false" tabindex="-1">Retained Raw ({ret_count}d)</button>
  <button type="button" role="tab" id="tab-btn-history" data-panel="panel-history" aria-selected="false" tabindex="-1">Historical ({hist_count}d)</button>
</div>

<div id="panel-live" role="tabpanel" aria-labelledby="tab-btn-live">
  <div class="panel-body">
    <div class="kpi-grid">
      <div class="kpi"><div class="kpi-label">CPU Load</div><div class="kpi-val">{cpu}</div></div>
      <div class="kpi"><div class="kpi-label">Memory Used</div><div class="kpi-val">{mem}</div></div>
      <div class="kpi"><div class="kpi-label">Temperature</div><div class="kpi-val">{temp}</div></div>
      <div class="kpi"><div class="kpi-label">Network</div><div class="kpi-val">{net_badge}</div></div>
    </div>
    <div class="toolbar">
      <span class="meta" id="win-label">Window:</span>
      <div class="tablist compact" role="tablist" aria-labelledby="win-label">
        <button type="button" role="tab" id="win-btn-12h" data-panel="win-12h" aria-selected="true" tabindex="0">Last 12h</button>
        <button type="button" role="tab" id="win-btn-24h" data-panel="win-24h" aria-selected="false" tabindex="-1">Last 24h</button>
      </div>
    </div>
    <div id="win-12h" role="tabpanel" aria-labelledby="win-btn-12h">
      <div class="chart-grid">"#,
        dir = escape_xml(data.log_dir_display),
        ret_count = data.retained.len(),
        hist_count = data.history.days_count,
        cpu = OptMetric(data.live.latest_cpu, "%"),
        mem = OptMetric(data.live.latest_mem, "%"),
        temp = OptMetric(data.live.latest_temp, "°C"),
    );
    write_chart_trio(&mut html, &data.live.window_12h);

    html.push_str(
        r#"      </div>
    </div>
    <div id="win-24h" role="tabpanel" aria-labelledby="win-btn-24h" hidden="until-found">
      <div class="chart-grid">"#,
    );
    write_chart_trio(&mut html, &data.live.window_24h);

    let _ = write!(
        html,
        r#"      </div>
    </div>
    <div class="chart-grid disk-section">
      {disk}
    </div>
  </div>
</div>

<div id="panel-retained" role="tabpanel" aria-labelledby="tab-btn-retained" hidden="until-found">
  <div class="panel-body">
"#,
        disk = data.live.disk_svg,
    );

    if data.retained.is_empty() {
        html.push_str(r#"    <p class="empty">No past raw-YYYY-MM-DD.log files found.</p>"#);
    } else {
        html.push_str(r#"    <div class="accordion">"#);
        for (idx, day) in data.retained.iter().rev().enumerate() {
            let open_attr = if idx == 0 { " open" } else { "" };
            let _ = write!(
                html,
                r#"      <details class="day-card"{open_attr}><summary>{date} | {samples} samples, {outages} offline</summary><div class="chart-grid">"#,
                date = escape_xml(&day.date_label),
                samples = day.samples,
                outages = day.outages,
            );
            write_chart_trio(&mut html, &day.charts);
            html.push_str("</div></details>\n");
        }
        html.push_str("    </div>\n");
    }

    let _ = write!(
        html,
        r#"  </div>
</div>

<div id="panel-history" role="tabpanel" aria-labelledby="tab-btn-history" hidden="until-found">
  <div class="panel-body">
    <div class="chart-grid">
      {hcpu}{hmem}{htemp}{hnet}
    </div>
  </div>
</div>
</main>
</div>
<script>{JS}</script>
</body>
</html>
"#,
        hcpu = data.history.cpu_svg,
        hmem = data.history.mem_svg,
        htemp = data.history.temp_svg,
        hnet = data.history.net_svg,
    );

    html
}
