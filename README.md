# beholder-brother

Minimal, near-zero-overhead Linux telemetry pair for low-spec self-hosted machines (Raspberry Pis, old laptops, SBCs).

- **`beholder`**: Headless daemon. Reads `/proc` and `/sys` directly, appends a line of text to a log, goes back to sleep. Once a day, crunches raw numbers into daily aggregates.
- **`brother`**: Companion visualizer. Reads the daily aggregates and renders a lightweight dashboard.

---

## How It Works

### 1. Sampling (`beholder`)
Every $N$ seconds (configurable, default `15s`), `beholder` wakes up, reads kernel stats via open file descriptors, appends a single line, and calls `nanosleep`:

```text
# timestamp cpu_user cpu_system cpu_idle mem_used_kb mem_total_kb temp_c
1731198000 12040 3410 894320 412000 1024000 48.2
1731198015 12055 3415 894460 413200 1024000 48.5
```

- No subprocess forks (`top`, `ps`, `free`).
- No heap allocations in the tick loop.
- No database engine or background worker threads.

### 2. Rollup (`beholder --rollup`)
Once every 24 hours (or triggered via CLI / `SIGHUP`), `beholder` rolls raw data into fixed daily buckets (min, avg, max, 95th percentile) and rotates the raw log:

```text
# date metric min avg max p95
2026-11-09 cpu_pct 0.8 4.2 88.1 12.0
2026-11-09 mem_pct 38.1 40.2 45.0 42.1
2026-11-09 temp_c  42.0 47.3 65.0 52.1
```

### 3. Visualizing (`brother`)
Reads rolled-up summaries and displays performance over days/weeks/months:
- **Embedded mode**: Single-binary HTTP listener on a local port (e.g. `http://<server-ip>:6060`) serving raw HTML/SVG. Zero JavaScript frameworks.
- **Remote sync mode**: Pull aggregate files over `rsync`/`ssh` to view locally on your laptop without running any web server on the monitored machine.

---

## Build & Run

```bash
cargo build --release
./target/release/beholder --interval 15s --log-dir /var/log/beholder
```
