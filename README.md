# beholder-brother

A lightweight pair of tools to monitor low-power Linux boxes (Raspberry Pis, old laptops, mini-PCs) without turning the monitoring tool into the heaviest process on the machine.

- **`beholder`**: The daemon. Wakes up, grabs a few numbers from `/proc` and `/sys`, checks if the internet is alive, appends a line to a plain text file, and goes back to sleep. Once a day, it rolls those raw lines into clean daily summaries.
- **`brother`**: The viewer. Reads the daily summaries and draws simple charts on a web page or right on your laptop.

---

## What We Collect (The Core 5)

We deliberately don't collect hundreds of metrics. If you need 500 different kernel gauges, use Netdata or Prometheus. We only track what actually matters on a home server:

1. **CPU Load %**: Sourced directly from `/proc/stat`.
2. **Memory Usage %**: Total vs Available memory from `/proc/meminfo`.
3. **Disk Usage %**: Space used on root (`/`) via the `statvfs` syscall.
4. **Temperature (°C)**: SoC / CPU thermal zone from `/sys/class/thermal/`.
5. **Network Connectivity (1/0)**: A quick, zero-dependency 500ms TCP probe (default `1.1.1.1:53`). No raw sockets, no root privileges, no DNS delays. If it connects, internet is up (`1`); if it times out, it's down (`0`).

---

## How It Works

### 1. Sampling (`beholder`)
Every $N$ milliseconds (default `10000ms` / 10s), `beholder` takes one sample and appends a single line:

```text
# timestamp cpu_pct mem_pct disk_pct temp_c net_ok
1731198000  4.2     38.5    62.1     48.2   1
1731198010  5.1     38.5    62.1     48.5   1
1731198020  4.8     38.6    62.1     48.3   0
```

- Zero child processes spawned (`top`, `free`, or `ping`).
- Zero memory allocations in the tick loop.
- Zero databases. Just plain append-only text.

### 2. Daily Rollup (`beholder --rollup`)
Once every 24 hours (or on demand), `beholder` crunches raw samples into daily aggregates (min, avg, max, 95th percentile) and resets the raw log so your storage doesn't fill up:

```text
# date       metric    min   avg   max   p95
2026-11-10   cpu_pct   0.8   4.2   88.1  12.0
2026-11-10   mem_pct   38.1  40.2  45.0  42.1
2026-11-10   disk_pct  62.1  62.1  62.1  62.1
2026-11-10   temp_c    42.0  47.3  65.0  52.1
2026-11-10   net_down_sec 0  30    --    --
```

*Downtime math is simple: count the `0` ticks in `net_ok` and multiply by interval seconds. No in-memory stopwatches.*

### 3. Visualizing (`brother`)
Reads the daily summaries:
- **Local web mode**: Starts a tiny HTTP listener on the server (e.g. `http://server-ip:6060`) serving plain HTML/SVG. No React, no Vue, no external CDN requests.
- **Laptop sync mode**: Sync the text logs to your laptop with `rsync` or `scp` and view them locally, keeping 0 bytes of web server overhead on the monitored server.

---

## Command Line Options

```text
Usage: beholder [OPTIONS]

Options:
    -h, --help              Print this help message
    -v, --version           Print version information
    -i, --interval <MS>     Sampling interval in ms (default: 10000)
    -l, --log-dir <PATH>    Directory for raw data (default: /var/log/beholder)
    -r, --rollup            Crunch existing logs into daily summaries and exit
    -n, --network <ADDR>    TCP address to probe for connectivity (default: 1.1.1.1:53)
```

## Build & Run

```bash
cargo build --release
./target/release/beholder-brother --interval 10000 --log-dir ./data
```
