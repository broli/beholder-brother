# ADR 001: Log Rotation, Retention, and Multi-Tier Storage Architecture

- **Status**: Accepted
- **Date**: 2026-11-12
- **Context**: `beholder-brother` Telemetry Storage & Retention Design

---

## 1. Problem Statement

A telemetry daemon sampling at 10-second intervals produces **8,640 samples per day** (~300 KB). Over months and years, this leads to two critical failure modes for low-spec self-hosted hardware:
1. **Flash Wear & Disk Bloat**: Continuously appending to an unbounded log wears down low-grade SD cards and USB drives, eventually exhausting disk space.
2. **Visualization Degradation**: A visualizer attempting to parse and render hundreds of thousands of raw data points over months will consume excessive CPU/RAM and freeze client browser tabs.

---

## 2. Core Decisions & Principles

### Decision 1: Two-Tier Storage (Fast Volatile vs. Slow Historical)

Telemetry is bifurcated into two independent storage tiers:

```
[Collector Daemon (10s)]
         │
         ▼
┌──────────────────────────────┐
│  Tier 1: High-Res Daily Logs │  `raw-YYYY-MM-DD.log`
│  (8,640 samples / ~300 KB)   │  Retained for N days (default: 7)
└──────────────┬───────────────┘
               │
               │ Midnight Rollup Pass
               ▼
┌──────────────────────────────┐
│  Tier 2: Historical Summary  │  `history.log`
│  (1 line per metric per day) │  Retained for M days (default: 180)
└──────────────────────────────┘
```

- **Tier 1 (High-Resolution Live Logs)**:
  - Filename format: `raw-YYYY-MM-DD.log`.
  - Captures 10-second volatile metrics (CPU, Memory, Thermals, Network reachability).
  - Purpose: Real-time monitoring and short-term post-crash forensic analysis.
  - Default retention: `7` days.
- **Tier 2 (Historical Rollup Database)**:
  - Filename: `history.log`.
  - Captures daily statistical reductions per metric: `min`, `avg`, `max`, `p95`, `net_down_sec`.
  - Purpose: Multi-month seasonal trend visualization.
  - Default retention: `180` days.

---

### Decision 2: Daily Segmented Files Over Single-File Truncation

**We explicitly reject maintaining a single monolithic `raw.log` that trims old lines.**

- **Rationale**: In POSIX filesystems, you cannot delete bytes from the beginning of a file without rewriting the entire file from scratch. Rewriting a multi-megabyte file daily creates severe I/O amplification on flash storage.
- **Implementation**: Each day writes to a distinct `raw-YYYY-MM-DD.log`. When a file exceeds the retention threshold, it is deleted via a single $O(1)$ `unlink()` system call, performing zero flash block rewrites.

---

### Decision 3: Retention Budgets & Ergonomics

| Configuration Flag | Default | Allowed Range | Technical Justification |
| :--- | :--- | :--- | :--- |
| **`--retain-days`** | `7` days | 1 – 30 days | 7 days covers a full week of 10-second diagnostics (~2.1 MB raw) without noticeable overhead. |
| **`--history-days`** | `180` days | 1 – 730 days | 180 days (~6 months) gives clear multi-season trends in `history.log` while staying ~29 KB. |
| **`--archive`** | `gzip` | `none`, `gzip`, `plain` | `gzip` compresses expired files to `archive/raw-YYYY-MM-DD.log.gz` (~35 KB) by default; `none` unlinks them; `plain` moves them uncompressed. |

---

### Decision 4: Storage Footprint

Under default settings (`--retain-days 7 --history-days 180 --archive gzip`):
- Active raw logs: $7 \times 300\text{ KB} \approx 2.1\text{ MB}$.
- Active disk logs: $7 \times 4\text{ KB} \approx 28\text{ KB}$.
- Historical summary: $180 \times 160\text{ bytes} \approx 28.8\text{ KB}$.
- Archived logs (`archive/*.gz`): grows by ~35 KB per retired day (~12.8 MB/year) because expired archives are not auto-pruned.
- **Strictly bounded mode (`--archive none`)**: Unlinks expired logs instead of archiving, keeping total disk usage **$< 2.5\text{ MB}$ permanently bounded**.

---

## 3. Consequences

- **Positive**:
  - Active `raw-*.log` and `history.log` are strictly bounded; with `--archive none`, the daemon can run for years without filling disks.
  - Zero-cost cleanup via `unlink()`.
  - Visualizer loads instantly because historical summaries never exceed ~30 KB.
- **Trade-offs**:
  - High-resolution (10s) data older than `retain_days` is permanently reduced to daily aggregates (or archived to `archive/*.gz` if `--archive gzip` is enabled, which grows ~12.8 MB/year unless pruned externally).

