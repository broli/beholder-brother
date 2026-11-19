# beholder-brother

Lightweight system telemetry daemon (`beholder`) and read-only HTML/SVG viewer (`brother`) for low-power Linux boxes. Plain-text logs, zero databases, zero frontend build steps.

## Binaries

- **`beholder`**: Samples CPU, memory, disk usage, SoC temperature, and TCP reachability into daily append-only logs, then rolls past days into `history.log` and archives/retires expired files.
- **`brother`**: Read-only HTTP server rendering a 3-tab dashboard (Live 12h/24h, Retained Raw, Historical) with inline SVGs and Catppuccin Latte/Mocha styling.

## Log Layout (`--log-dir`)

- `raw-YYYY-MM-DD.log` — `# timestamp cpu_pct mem_pct temp_c net_ok interval_ms`
- `disks-YYYY-MM-DD.log` — `# timestamp mount used_pct`
- `history.log` — `# date metric min avg max p95` (`cpu_pct`, `mem_pct`, `temp_c`, `net_down_sec`)
- `archive/` — retired logs when `--archive` is `gzip` (default) or `plain`

## Build & Test

```bash
cargo build --release
# binaries: target/release/beholder and target/release/brother

just test          # cargo fmt --check, clippy -D warnings, cargo test
just pedantic-test # clippy pedantic + nursery
```

## Usage

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

For remote machines, sync `/var/log/beholder` locally via `rsync`/`scp` or bind `brother` to localhost and tunnel over SSH/WireGuard.

## Notes & Known Limitations

- **`Ctrl+C` shutdown in `beholder`**: Right now `beholder` sleeps for the full `--interval` between ticks, so hitting `Ctrl+C` can wait up to one interval before exiting. I don't know how to handle this situation cleanly yet—I'll fix it in a future release. For now it works.
- **Thermal zones**: Currently only reads `/sys/class/thermal/thermal_zone0/temp`. Supporting multiple thermal zones / `hwmon` sensors while keeping the lightweight zero-bloat philosophy is on the plan for the future.
- **Single-threaded HTTP (`brother`)**: By design philosophy, I prefer the app to stay single-threaded even if a slow client can briefly stall another browser refresh. Keeping it simple and tiny matters more here than concurrent request handling.
