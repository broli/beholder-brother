use crate::brother::svg::escape_xml;
use std::fmt::Write as _;

pub struct LiveSummary {
    pub latest_cpu: Option<f32>,
    pub latest_mem: Option<f32>,
    pub latest_temp: Option<f32>,
    pub latest_net_ok: Option<bool>,
    pub disk_svg: String,
    pub cpu_12h_svg: String,
    pub mem_12h_svg: String,
    pub temp_12h_svg: String,
    pub cpu_24h_svg: String,
    pub mem_24h_svg: String,
    pub temp_24h_svg: String,
}

#[derive(Clone)]
pub struct RetainedDayView {
    pub date_label: String,
    pub samples: usize,
    pub outages: usize,
    pub cpu_svg: String,
    pub mem_svg: String,
    pub temp_svg: String,
}

#[derive(Clone)]
pub struct HistoryView {
    pub days_count: usize,
    pub cpu_svg: String,
    pub mem_svg: String,
    pub temp_svg: String,
    pub net_svg: String,
}

pub struct DashboardData {
    pub log_dir_display: String,
    pub scale: f32,
    pub live: LiveSummary,
    pub retained: Vec<RetainedDayView>,
    pub history: HistoryView,
}

const CSS: &str = r#"
:root {
  --bg: #eff1f5;
  --card: #e6e9ef;
  --surface: #ffffff;
  --text: #4c4f69;
  --subtext: #6c6f85;
  --overlay: #bcc0cc;
  --grid: #dce0e8;
  --shadow: #ccd0da;
  --blue: #1e66f5;
  --green: #40a02b;
  --peach: #fe640b;
  --red: #d20f39;
  --mauve: #8839ef;
}
@media (prefers-color-scheme: dark) {
  :root {
    --bg: #1e1e2e;
    --card: #181825;
    --surface: #313244;
    --text: #cdd6f4;
    --subtext: #a6adc8;
    --overlay: #585b70;
    --grid: #45475a;
    --shadow: #11111b;
    --blue: #89b4fa;
    --green: #a6e3a1;
    --peach: #fab387;
    --red: #f38ba8;
    --mauve: #cba6f7;
  }
}
* { box-sizing: border-box; }
body {
  zoom: var(--scale, 1);
  margin: 0;
  padding: 1.5rem;
  font-family: system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
  background: var(--bg);
  color: var(--text);
  line-height: 1.45;
}
.wrap { max-width: 1120px; margin: 0 auto; }
header {
  display: flex;
  flex-wrap: wrap;
  justify-content: space-between;
  align-items: center;
  gap: 1rem;
  margin-bottom: 1.25rem;
}
h1 { margin: 0; font-size: 1.4rem; letter-spacing: -0.02em; }
.meta { font-size: 0.85rem; color: var(--subtext); font-family: ui-monospace, monospace; }
.tabs { display: flex; gap: 0.5rem; flex-wrap: wrap; margin-bottom: 1.25rem; }
.tab-btn, .win-btn {
  border: 1px solid var(--overlay);
  background: var(--surface);
  color: var(--text);
  padding: 0.45rem 0.9rem;
  border-radius: 8px;
  cursor: pointer;
  font-weight: 600;
  font-size: 0.9rem;
  box-shadow: 2px 2px 0 var(--shadow);
  transition: transform 0.12s ease, background 0.12s ease;
}
.tab-btn:hover, .win-btn:hover { transform: translateY(-1px); }
.tab-btn.active, .win-btn.active {
  background: var(--blue);
  color: var(--bg);
  border-color: var(--blue);
}
.panel { display: none; }
.panel.active { display: block; }
.kpi-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(170px, 1fr));
  gap: 0.9rem;
  margin-bottom: 1.25rem;
}
.kpi {
  background: var(--surface);
  border: 1px solid var(--overlay);
  border-radius: 10px;
  padding: 0.85rem 1rem;
  box-shadow: 4px 4px 0 var(--shadow);
}
.kpi-label { font-size: 0.78rem; color: var(--subtext); text-transform: uppercase; letter-spacing: 0.04em; }
.kpi-val { font-size: 1.45rem; font-weight: 700; margin-top: 0.2rem; font-family: ui-monospace, monospace; }
.chart-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(460px, 1fr));
  gap: 1.1rem;
}
.chart-svg { width: 100%; height: auto; display: block; }
.toolbar { display: flex; align-items: center; gap: 0.5rem; margin-bottom: 1rem; }
.win-pane { display: none; }
.win-pane.active { display: grid; }
details.day-card {
  background: var(--card);
  border: 1px solid var(--overlay);
  border-radius: 10px;
  padding: 0.75rem 1rem;
  margin-bottom: 0.9rem;
  box-shadow: 4px 4px 0 var(--shadow);
}
details.day-card summary {
  cursor: pointer;
  font-weight: 600;
  font-family: ui-monospace, monospace;
}
details.day-card[open] summary { margin-bottom: 0.85rem; }
.empty { color: var(--subtext); font-style: italic; }
"#;

const JS: &str = r#"
document.querySelectorAll('.tab-btn').forEach(btn => {
  btn.addEventListener('click', () => {
    document.querySelectorAll('.tab-btn').forEach(b => b.classList.remove('active'));
    document.querySelectorAll('.panel').forEach(p => p.classList.remove('active'));
    btn.classList.add('active');
    document.getElementById(btn.dataset.tab).classList.add('active');
  });
});
document.querySelectorAll('.win-btn').forEach(btn => {
  btn.addEventListener('click', () => {
    document.querySelectorAll('.win-btn').forEach(b => b.classList.remove('active'));
    document.querySelectorAll('.win-pane').forEach(p => p.classList.remove('active'));
    btn.classList.add('active');
    document.getElementById(btn.dataset.win).classList.add('active');
  });
});
"#;

fn fmt_opt(val: Option<f32>, suffix: &str) -> String {
    match val {
        Some(v) => format!("{v:.1}{suffix}"),
        None => "-".to_string(),
    }
}

pub fn render_page(data: &DashboardData) -> String {
    let mut html = String::with_capacity(32 * 1024);
    let scale = data.scale;
    let net_badge = match data.live.latest_net_ok {
        Some(true) => r#"<span style="color:var(--green)">ONLINE</span>"#,
        Some(false) => r#"<span style="color:var(--red)">DOWN</span>"#,
        None => "-",
    };

    let _ = write!(
        html,
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>brother — beholder telemetry</title>
<style>{CSS}
:root {{ --scale: {scale:.2}; }}
</style>
</head>
<body>
<div class="wrap">
<header>
  <h1>brother</h1>
  <div class="meta">log-dir: {dir}</div>
</header>

<nav class="tabs" aria-label="Dashboard sections">
  <button class="tab-btn active" data-tab="tab-live">Live (Now)</button>
  <button class="tab-btn" data-tab="tab-retained">Retained Raw ({ret_count}d)</button>
  <button class="tab-btn" data-tab="tab-history">Historical ({hist_count}d)</button>
</nav>

<section id="tab-live" class="panel active">
  <div class="kpi-grid">
    <div class="kpi"><div class="kpi-label">CPU Load</div><div class="kpi-val">{cpu}</div></div>
    <div class="kpi"><div class="kpi-label">Memory Used</div><div class="kpi-val">{mem}</div></div>
    <div class="kpi"><div class="kpi-label">Temperature</div><div class="kpi-val">{temp}</div></div>
    <div class="kpi"><div class="kpi-label">Network</div><div class="kpi-val">{net_badge}</div></div>
  </div>
  <div class="toolbar">
    <span class="meta">Window:</span>
    <button class="win-btn active" data-win="win-12h">Last 12h</button>
    <button class="win-btn" data-win="win-24h">Last 24h</button>
  </div>
  <div id="win-12h" class="chart-grid win-pane active">
    {c12}{m12}{t12}
  </div>
  <div id="win-24h" class="chart-grid win-pane">
    {c24}{m24}{t24}
  </div>
  <div class="chart-grid" style="margin-top:1.1rem">
    {disk}
  </div>
</section>

<section id="tab-retained" class="panel">
"#,
        dir = escape_xml(&data.log_dir_display),
        ret_count = data.retained.len(),
        hist_count = data.history.days_count,
        cpu = fmt_opt(data.live.latest_cpu, "%"),
        mem = fmt_opt(data.live.latest_mem, "%"),
        temp = fmt_opt(data.live.latest_temp, "°C"),
        c12 = data.live.cpu_12h_svg,
        m12 = data.live.mem_12h_svg,
        t12 = data.live.temp_12h_svg,
        c24 = data.live.cpu_24h_svg,
        m24 = data.live.mem_24h_svg,
        t24 = data.live.temp_24h_svg,
        disk = data.live.disk_svg,
    );

    if data.retained.is_empty() {
        html.push_str(r#"<p class="empty">No past raw-YYYY-MM-DD.log files found.</p>"#);
    } else {
        for (idx, day) in data.retained.iter().rev().enumerate() {
            let open_attr = if idx == 0 { " open" } else { "" };
            let _ = writeln!(
                html,
                r#"<details class="day-card"{open_attr}><summary>{date} — {samples} samples, {outages} network drops</summary><div class="chart-grid">{cpu}{mem}{temp}</div></details>"#,
                date = escape_xml(&day.date_label),
                samples = day.samples,
                outages = day.outages,
                cpu = day.cpu_svg,
                mem = day.mem_svg,
                temp = day.temp_svg,
            );
        }
    }

    let _ = write!(
        html,
        r#"</section>

<section id="tab-history" class="panel">
  <div class="chart-grid">
    {hcpu}{hmem}{htemp}{hnet}
  </div>
</section>
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
