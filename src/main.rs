use lexopt::prelude::*;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

mod savelog;
mod sense;

struct Config {
    interval_ms: u64,
    log_dir: PathBuf,
    rollup: bool,
    network_target: SocketAddr,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            interval_ms: 10000,
            log_dir: PathBuf::from("/var/log/beholder"),
            rollup: false,
            network_target: SocketAddr::from(([1, 1, 1, 1], 53)),
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let running = Arc::new(AtomicBool::new(true));
    //not cloning, since i used ARC, this is a second pointer to the same heap space
    let r = running.clone();

    ctrlc::set_handler(move || {
        eprintln!("\nInterrupt received, shutting down...");
        r.store(false, Ordering::SeqCst);
    })?;

    // read command args
    let config = match parse_args()? {
        Some(cfg) => cfg,
        None => return Ok(()),
    };

    let mut prev_cpu = sense::PreviousCpuTick::new();

    while running.load(Ordering::SeqCst) {
        // read sensor data
        let sample = sense::sense(&config, &mut prev_cpu)?;
        println!("Sample: {:?}", sample);
        // save data in temporal log
        // either here or in sense.rs

        // nap for configured time
        std::thread::sleep(std::time::Duration::from_millis(config.interval_ms));
    }
    println!("\nDaemon stopped  \"cleanly\".");
    Ok(())
}

fn print_usage() {
    println!(
        r#"Usage: beholder [OPTIONS]

Options:
    -h, --help              Print this help message
    -v, --version           Print version information
    -i, --interval <MS>     Sampling interval in ms (default: 10000)
    -l, --log-dir <PATH>    Directory for raw data (default: /var/log/beholder)
    -r, --rollup            Crunch existing logs into daily summaries and exit
    -n, --network <ADDR>    TCP address to probe for connectivity (default: 1.1.1.1:53)

Sensors:
    - CPU load %
    - Memory usage %
    - Disk usage %
    - Temperature (°C)
    - Network uptime (probe 1/0)
"#
    );
}

fn print_version() {
    println!("Beholder Version {}", env!("CARGO_PKG_VERSION"));
}

fn parse_args() -> Result<Option<Config>, lexopt::Error> {
    let mut parser = lexopt::Parser::from_env();
    let mut cfg = Config::default();

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
                cfg.interval_ms = parser.value()?.parse()?;
            }
            Short('l') | Long("log-dir") => {
                cfg.log_dir = PathBuf::from(parser.value()?);
            }
            Short('r') | Long("rollup") => {
                cfg.rollup = true;
            }
            Short('n') | Long("network") => {
                cfg.network_target = parser.value()?.parse()?;
            }
            _ => return Err(arg.unexpected()),
        }
    }
    Ok(Some(cfg))
}
