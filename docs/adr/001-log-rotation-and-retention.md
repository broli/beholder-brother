# ADR 001: Log Rotation, Retention, and Two-Tier Storage

- **Status**: Accepted
- **Date**: 2026-11-12
- **Context**: `beholder-brother` telemetry storage and retention

## Problem

At the default 10-second sampling interval, `beholder` writes 8,640 samples per day (~300 KB). Appending indefinitely to a single log file causes two issues on small Linux hosts:

1. Rewriting or truncating the head of a monolithic file forces full-file rewrites, causing unnecessary flash wear on SD cards and USB storage.
2. Parsing months of raw 10-second samples on every dashboard build slows down `brother` and bloats the rendered SVGs.

## Decisions

### 1. Two-tier storage

- **Tier 1 (`raw-YYYY-MM-DD.log`, `disks-YYYY-MM-DD.log`)**: High-resolution daily logs retained for `--retain-days` (default `7`). Used for live views and short-term debugging.
- **Tier 2 (`history.log`)**: Daily statistical summaries (`min`, `avg`, `max`, `p95` for CPU, memory, and temperature, plus `net_down_sec`) retained for `--history-days` (default `180`).

### 2. Daily segmented files

Each UTC day writes to a separate `raw-YYYY-MM-DD.log` and `disks-YYYY-MM-DD.log`. Past days are rolled into `history.log` once, and files older than `retain_days` are either compressed to `archive/<file>.gz` (`--archive gzip`, default), moved to `archive/` (`--archive plain`), or removed via `unlink` (`--archive none`). Expired entries in `history.log` are pruned past `history_days`.

### 3. Configuration defaults

| Flag | Default | Type | Effect |
| :--- | :--- | :--- | :--- |
| `--retain-days` | `7` | `u64` | Days to keep raw and disk daily logs before retirement |
| `--history-days` | `180` | `u64` | Days to retain in `history.log` |
| `--archive` | `gzip` | `none \| gzip \| plain` | Action taken on expired daily logs |

## Storage Footprint

With default settings (`--retain-days 7 --history-days 180 --archive gzip`):

- Active raw logs: ~2.1 MB (`7 * ~300 KB`)
- Active disk logs: ~28 KB (`7 * ~4 KB`)
- `history.log`: ~29 KB (`180 * ~160 B`)
- `archive/*.gz`: grows by ~35 KB per retired day unless pruned externally; `--archive none` keeps total disk usage bounded under ~2.5 MB.
