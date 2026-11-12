use crate::sense::{DiskSample, Sample};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use time::OffsetDateTime;
use time::macros::format_description;

pub fn append_sample(log_dir: &Path, sample: &Sample) -> Result<(), Box<dyn std::error::Error>> {
    let mut file = open_daily_log(
        log_dir,
        "raw",
        sample.timestamp,
        "# timestamp cpu_pct mem_pct temp_c net_ok",
    )?;

    write!(
        file,
        "{} {:.2} {:.2} ",
        sample.timestamp, sample.cpu_pct, sample.mem_pct
    )?;
    match sample.temp_c {
        Some(t) => write!(file, "{:.1}", t)?,
        None => write!(file, "-")?,
    }
    writeln!(file, " {}", u8::from(sample.net_ok))?;

    Ok(())
}

pub fn append_disks(
    log_dir: &Path,
    timestamp: u64,
    disks: &[DiskSample],
) -> Result<(), Box<dyn std::error::Error>> {
    let mut file = open_daily_log(log_dir, "disks", timestamp, "# timestamp mount used_pct")?;

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

fn sample_timestamp(timestamp: u64) -> Result<String, Box<dyn std::error::Error>> {
    let dt = OffsetDateTime::from_unix_timestamp(timestamp as i64)?;
    let format = format_description!("[year]-[month]-[day]");
    Ok(dt.format(&format)?)
}

fn open_daily_log(
    log_dir: &Path,
    prefix: &str,
    timestamp: u64,
    header: &str,
) -> Result<std::fs::File, Box<dyn std::error::Error>> {
    fs::create_dir_all(log_dir)?;
    let date_str = sample_timestamp(timestamp)?;
    let file_path = log_dir.join(format!("{}-{}.log", prefix, date_str));

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&file_path)?;

    if file.metadata()?.len() == 0 {
        writeln!(file, "{}", header)?;
    }

    Ok(file)
}
