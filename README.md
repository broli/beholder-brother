# beholder-brother

The name of this project is a wordplay on the old bb, but it does not work like it 

Its a lightweight system telemetry daemon (`beholder`) and read-only visualizer (`brother`) for low-power Linux boxes. Plain-text logs, zero databases, zero frontend build steps. As little dependencies as possible. 

## Binaries

- **`beholder`**: Samples CPU, memory, disk usage, SoC temperature, and TCP reachability into daily append-only logs, then rolls past days into `history.log` and archives/retires expired files.
- **`brother`**: Read-only HTTP server rendering the dashboard (Live 12h/24h, Retained Raw, Historical) with inline SVGs and light/dark theming ~stolen~ borrowed() from the internet.

## Log Layout (`--log-dir`)

- `raw-YYYY-MM-DD.log`: `# timestamp cpu_pct mem_pct temp_c net_ok interval_ms`
- `disks-YYYY-MM-DD.log`: `# timestamp mount used_pct`
- `history.log`: `# date metric min avg max p95` (`cpu_pct`, `mem_pct`, `temp_c`, `net_down_sec`)
- `archive/`: retired logs when `--archive` is `gzip` (default) or `plain`

## Build & Test

```bash
cargo build --release
# binaries: target/release/beholder and target/release/brother

just test          # cargo fmt --check, clippy -D warnings, cargo test
just pedantic-test # clippy pedantic + nursery
```

## Usage
I would recommend you set the --interval to 60000
If you are trying to catch a specific issue, use a smaller interval and --flush 

### `beholder`

```text
Usage: beholder [OPTIONS]

Options:
    -h, --help                  Print help
    -v, --version               Print version
    -i, --interval <MS>         Sampling interval in ms (default: 10000)
    -l, --log-dir <PATH>        Log directory (default: /var/log/beholder)
    -r, --retain-days <DAYS>    Days to keep raw/disk logs (default: 7)
    -H, --history-days <DAYS>   Days to keep in history.log (default: 180)
    -A, --archive <TYPE>        none | gzip (default) | plain
    -n, --network <ADDR>        TCP probe target (default: 1.1.1.1:53)
    -m, --mount <PATH>          Mount point to track; repeatable (default: /)
    -F, --flush                 Flush every sample immediately (default: 4 KB buffer)
```

### `brother`

```text
Usage: brother [OPTIONS]

Options:
    -h, --help              Print help
    -v, --version           Print version
    -l, --log-dir <PATH>    Log directory to read (default: /var/log/beholder)
    -b, --bind <ADDR>       Listen address (default: 127.0.0.1:6060)
    -f, --force             Bypass SVG cache and reload from disk per request
    -s, --scale <FACTOR>    Page zoom factor in 0.5..=4.0 (default: 1.0)
```
## Notes & Known Limitations

- **Thermal zones**: Reads `/sys/class/thermal/thermal_zone0/temp` only. Multi-zone / `hwmon` support is not implemented yet.
- **Single-threaded HTTP (`brother`)**: Serves requests sequentially on one thread to keep the binary and runtime footprint small; a stalled client briefly delays other requests.
