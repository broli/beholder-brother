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
│  (8,640 samples / ~300 KB)   │  Retained for N days (default: 2)
└──────────────┬───────────────┘
               │
               │ Midnight Rollup Pass (`beholder --rollup`)
               ▼
┌──────────────────────────────┐
│  Tier 2: Historical Summary  │  `history.log`
│  (1 line per metric per day) │  Retained for M days (default: 90)
└──────────────────────────────┘
```

- **Tier 1 (High-Resolution Live Logs)**:
  - Filename format: `raw-YYYY-MM-DD.log`.
  - Captures 10-second volatile metrics (CPU, Memory, Thermals, Network reachability).
  - Purpose: Real-time monitoring and short-term post-crash forensic analysis.
- **Tier 2 (Historical Rollup Database)**:
  - Filename: `history.log`.
  - Captures daily statistical reductions per metric: `min`, `avg`, `max`, `p95`, `net_down_sec`.
  - Purpose: Multi-month seasonal trend visualization.

---

### Decision 2: Daily Segmented Files Over Single-File Truncation

**We explicitly reject maintaining a single monolithic `raw.log` that trims old lines.**

- **Rationale**: In POSIX filesystems, you cannot delete bytes from the beginning of a file without rewriting the entire file from scratch. Rewriting a multi-megabyte file daily creates severe I/O amplification on flash storage.
- **Implementation**: Each day writes to a distinct `raw-YYYY-MM-DD.log`. When a file exceeds the retention threshold, it is deleted via a single $O(1)$ `unlink()` system call, performing zero flash block rewrites.

---

### Decision 3: Retention Budgets & Ergonomics

| Configuration Flag | Default | Allowed Range | Technical Justification |
| :--- | :--- | :--- | :--- |
| **`--retain-days`** | `2` days | 1 – 30 days (Warn if $>7$) | 2 days handles weekend power outages while keeping raw logs under 1 MB. $>7$ days ($>60,000$ points) risks client rendering lag on low-power devices. |
| **`--history-days`** | `90` days | 1 – 730 days | 90 days (~3 months) allocates ~10 pixels per day on a standard 1080p display, enabling clear bar/point legibility without complex pan/zoom controls. |
| **`--archive`** | `none` | `none`, `gzip` | `none` unlinks expired daily files. `gzip` compresses expired files to `raw-YYYY-MM-DD.log.gz` (~35 KB) for operators who want deep archival. |

---

### Decision 4: Storage Footprint Guarantees

Under default settings (`--retain-days 2 --history-days 90 --archive none`):
- Active raw logs: $2 \times 300\text{ KB} = 600\text{ KB}$.
- Historical summary: $90 \times 160\text{ bytes} \approx 14.4\text{ KB}$.
- Disks log: $90 \text{ days} \times 144 \text{ entries} \approx 350\text{ KB}$.
- **Total Maximum Disk Footprint**: **$< 1.5\text{ MB}$ permanently bounded**.

---

## 3. Consequences

- **Positive**:
  - The storage budget is mathematically bounded; the daemon can run for years without filling disks.
  - Zero-cost cleanup via `unlink()`.
  - Visualizer loads instantly because historical summaries never exceed ~15–30 KB.
- **Trade-offs**:
  - High-resolution (10s) data older than `retain_days` is permanently reduced to daily aggregates (or archived to `.gz` if enabled).

