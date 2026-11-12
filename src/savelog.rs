use crate::sense::{DiskSample, Sample};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

pub fn append_sample(log_dir: &Path, sample: &Sample) -> Result<(), Box<dyn std::error::Error>> {
    fs::create_dir_all(log_dir)?;

    let file_path = log_dir.join("raw.log");
    let is_new = !file_path.exists();

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&file_path)?;

    // If the file is brand new, write a header comment!
    if is_new {
        writeln!(file, "# timestamp cpu_pct mem_pct temp_c net_ok")?;
    }

    let temp_str = match sample.temp_c {
        Some(t) => format!("{:.1}", t),
        None => "-".to_string(),
    };
    let net_val = if sample.net_ok { 1 } else { 0 };

    writeln!(
        file,
        "{} {:.2} {:.2} {} {}",
        sample.timestamp, sample.cpu_pct, sample.mem_pct, temp_str, net_val
    )?;

    Ok(())
}

pub fn append_disks(
    log_dir: &Path,
    timestamp: u64,
    disks: &[DiskSample],
) -> Result<(), Box<dyn std::error::Error>> {
    fs::create_dir_all(log_dir)?;

    let file_path = log_dir.join("disks.log");
    let is_new = !file_path.exists();

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&file_path)?;

    if is_new {
        writeln!(file, "# timestamp mount used_pct")?;
    }

    for disk in disks {
        writeln!(
            file,
            "{} {} {:.2}",
            timestamp,
            disk.mount.display(),
            disk.used_pct
        )?;
    }

    Ok(())
}
