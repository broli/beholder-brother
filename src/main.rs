use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

mod savelog;
mod sense;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let running = Arc::new(AtomicBool::new(true));
    //not cloning, since i used ARC, this is a second pointer to the same heap space
    let r = running.clone();

    ctrlc::set_handler(move || {
        r.store(false, Ordering::SeqCst);
    })?;

    // read command args

    while running.load(Ordering::SeqCst) {
        // read sensor data
        // execute sense.rs
        // save data in temporal log
        // either here or in sense.rs

        // nap for configured time
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
