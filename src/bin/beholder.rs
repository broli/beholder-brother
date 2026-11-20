#![forbid(unsafe_code)]

use beholder_brother::{ArchiveType, Config, Error, Result, rollout, savelog, sense, today_utc};
use lexopt::prelude::*;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

fn main() -> Result<()> {
    let Some(config) = parse_args()? else {
        return Ok(());
    };

    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();
    let main_thread = std::thread::current();

    ctrlc::set_handler(move || {
        eprintln!("\nInterrupt received, shutting down...");
        r.store(false, Ordering::Relaxed);
        main_thread.unpark();
    })?;

    let mut prev_cpu = sense::PreviousCpuTick::default();
    sense::prime_cpu(&mut prev_cpu)?;
    sleep_interruptible(
        &running,
        Duration::from_millis(config.interval_ms.clamp(50, 200)),
    );

    let mut logs = savelog::DailyLogs::new(config.log_dir.clone(), config.flush_each_sample);
    let mut tick: u64 = 0;
    let mut last_rotate_day = None;

    while running.load(Ordering::Relaxed) {
        match sense::sense(&config, &mut prev_cpu) {
            Ok(sample) => {
                logs.append_sample(&sample, config.interval_ms)?;
                if tick == 0 {
                    match sense::sense_disks(&config) {
                        Ok(disk_samples) => logs.append_disks(sample.timestamp, &disk_samples)?,
                        Err(err) => eprintln!("Disk sense error: {err}"),
                    }
                }
            }
            Err(err) => eprintln!("Sample error: {err}"),
        }

        let today = today_utc()?;
        if last_rotate_day != Some(today) {
            logs.flush()?;
            if let Err(err) = rollout::rollup_and_retire(&config, &today) {
                eprintln!("Rollup/retire error for {today}: {err}");
            }
            last_rotate_day = Some(today);
        }

        tick = (tick + 1) % 60;
        sleep_interruptible(&running, Duration::from_millis(config.interval_ms));
    }

    logs.flush()?;
    println!("\nDaemon stopped cleanly.");
    Ok(())
}

fn sleep_interruptible(running: &AtomicBool, duration: Duration) {
    let deadline = Instant::now() + duration;
    while running.load(Ordering::Relaxed) {
        let now = Instant::now();
        if now >= deadline {
            break;
        }
        std::thread::park_timeout(deadline - now);
    }
}

fn print_usage() {
    println!(
        r"Usage: beholder [OPTIONS]

Options:
    -h, --help                  Print this help message
    -v, --version               Print version information
    -i, --interval <MS>         Sampling interval in ms (default: 10000)
    -l, --log-dir <PATH>        Directory for raw data (default: /var/log/beholder)
    -r, --retain-days <DAYS>    Days to retain daily detailed logs (default: 7)
    -H, --history-days <DAYS>   Days to retain history logs (default: 180)
    -A, --archive <TYPE>        none | gzip (default) | plain
    -n, --network <ADDR>        TCP address to probe for connectivity (default: 1.1.1.1:53)
    -m, --mount <PATH>          Mount point to track; repeatable (default: /)
    -F, --flush                 Flush every sample immediately to disk (disables 4 KB write buffer)
"
    );
}

fn print_version() {
    println!("Beholder Version {}", env!("CARGO_PKG_VERSION"));
}

fn parse_args() -> Result<Option<Config>> {
    let mut parser = lexopt::Parser::from_env();
    let mut cfg = Config::default();
    let mut custom_mounts = false;

    while let Some(arg) = parser.next()? {
        match arg {
            Short('h') | Long("help") => {
                print_usage();
                return Ok(None);
            }
            Short('v') | Long("version") => {
                print_version();
                return Ok(None);
            }
            Short('i') | Long("interval") => {
                let ms: u64 = parser.value()?.parse()?;
                if ms == 0 {
                    return Err(Error::InvalidArg(
                        "interval must be greater than 0 ms".into(),
                    ));
                }
                cfg.interval_ms = ms;
            }
            Short('l') | Long("log-dir") => {
                cfg.log_dir = PathBuf::from(parser.value()?);
                cfg.archive_dir = cfg.log_dir.join("archive");
            }
            Short('r') | Long("retain-days") => {
                cfg.retain_days = parser.value()?.parse()?;
            }
            Short('H') | Long("history-days") => {
                cfg.history_days = parser.value()?.parse()?;
            }
            Short('A') | Long("archive") => {
                let archive_type = parser.value()?;
                cfg.archive_type = match archive_type.to_str() {
                    Some("none") => ArchiveType::None,
                    Some("gzip") => ArchiveType::Gzip,
                    Some("plain") => ArchiveType::Plain,
                    _ => {
                        return Err(Error::InvalidArg(format!(
                            "invalid archive type: {}",
                            archive_type.to_string_lossy()
                        )));
                    }
                };
            }
            Short('n') | Long("network") => {
                cfg.network_target = parser.value()?.parse()?;
            }
            Short('m') | Long("mount") => {
                if !custom_mounts {
                    custom_mounts = true;
                    cfg.mounts.clear();
                }
                let path = PathBuf::from(parser.value()?);
                if !cfg.mounts.contains(&path) {
                    cfg.mounts.push(path);
                }
            }
            Short('F') | Long("flush") => {
                cfg.flush_each_sample = true;
            }
            _ => return Err(arg.unexpected().into()),
        }
    }
    Ok(Some(cfg))
}
