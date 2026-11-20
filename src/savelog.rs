use crate::sense::{DiskSample, Sample};
use crate::{Result, timestamp_to_utc_date};
use std::fmt::Write as _;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use time::Date;

const BUFFER_BYTES: usize = 4096;
const RAW_HEADER: &str = "# timestamp cpu_pct mem_pct temp_c net_ok interval_ms";
const DISKS_HEADER: &str = "# timestamp mount used_pct";

struct LogStream {
    prefix: &'static str,
    header: &'static str,
    active_date: Option<Date>,
    writer: Option<BufWriter<File>>,
}

impl LogStream {
    const fn new(prefix: &'static str, header: &'static str) -> Self {
        Self {
            prefix,
            header,
            active_date: None,
            writer: None,
        }
    }

    fn write_line(
        &mut self,
        log_dir: &Path,
        date: Date,
        line: &str,
        flush_each: bool,
    ) -> Result<()> {
        let writer = if self.active_date == Some(date)
            && let Some(writer) = self.writer.as_mut()
        {
            writer
        } else {
            if let Some(prev) = self.writer.as_mut() {
                prev.flush()?;
            }
            let file = open_daily_log(log_dir, self.prefix, date, self.header)?;
            self.active_date = Some(date);
            self.writer
                .insert(BufWriter::with_capacity(BUFFER_BYTES, file))
        };

        writer.write_all(line.as_bytes())?;
        if flush_each {
            writer.flush()?;
        }
        Ok(())
    }

    fn flush(&mut self) -> io::Result<()> {
        if let Some(writer) = self.writer.as_mut() {
            writer.flush()?;
        }
        Ok(())
    }
}

pub struct DailyLogs {
    log_dir: PathBuf,
    flush_each_sample: bool,
    raw: LogStream,
    disks: LogStream,
    buf: String,
}

impl DailyLogs {
    pub fn new(log_dir: PathBuf, flush_each_sample: bool) -> Self {
        Self {
            log_dir,
            flush_each_sample,
            raw: LogStream::new("raw", RAW_HEADER),
            disks: LogStream::new("disks", DISKS_HEADER),
            buf: String::with_capacity(128),
        }
    }

    pub fn append_sample(&mut self, sample: &Sample, interval_ms: u64) -> Result<()> {
        let date = timestamp_to_utc_date(sample.timestamp)?;
        self.buf.clear();
        let _ = write!(
            self.buf,
            "{} {:.2} {:.2} ",
            sample.timestamp, sample.cpu_pct, sample.mem_pct
        );
        match sample.temp_c {
            Some(t) => {
                let _ = write!(self.buf, "{t:.1}");
            }
            None => self.buf.push('-'),
        }
        let _ = writeln!(self.buf, " {} {}", u8::from(sample.net_ok), interval_ms);

        self.raw
            .write_line(&self.log_dir, date, &self.buf, self.flush_each_sample)?;
        Ok(())
    }

    pub fn append_disks(&mut self, timestamp: u64, disks: &[DiskSample]) -> Result<()> {
        let date = timestamp_to_utc_date(timestamp)?;
        self.buf.clear();
        for disk in disks {
            let _ = writeln!(
                self.buf,
                "{timestamp} {} {:.2}",
                disk.mount.display(),
                disk.used_pct
            );
        }
        if self.buf.is_empty() {
            return Ok(());
        }

        self.disks
            .write_line(&self.log_dir, date, &self.buf, self.flush_each_sample)?;
        Ok(())
    }

    pub fn flush(&mut self) -> Result<()> {
        self.raw.flush()?;
        self.disks.flush()?;
        Ok(())
    }
}

impl Drop for DailyLogs {
    fn drop(&mut self) {
        let _ = self.flush();
    }
}

fn open_daily_log(log_dir: &Path, prefix: &str, date: Date, header: &str) -> Result<File> {
    fs::create_dir_all(log_dir)?;
    let file_path = log_dir.join(format!("{prefix}-{date}.log"));

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&file_path)?;

    if file.metadata()?.len() == 0 {
        writeln!(file, "{header}")?;
    }

    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestTempDir;

    fn sample(ts: u64, temp_c: Option<f32>) -> Sample {
        Sample {
            timestamp: ts,
            cpu_pct: 12.5,
            mem_pct: 45.0,
            temp_c,
            net_ok: true,
        }
    }

    #[test]
    fn test_daily_logs_buffered_flush_and_midnight_rotation() -> Result<()> {
        let dir = TestTempDir::new("savelog_buf")?;
        let day1_ts = 1_794_355_200; // 2026-11-11 00:00:00 UTC
        let day2_ts = day1_ts + 86_400; // 2026-11-12 00:00:00 UTC

        {
            let mut logs = DailyLogs::new(dir.path().to_path_buf(), false);
            logs.append_sample(&sample(day1_ts, Some(48.2)), 10_000)?;
            logs.append_sample(&sample(day1_ts + 10, None), 10_000)?;
            // Rotating to day 2 flushes day 1 cleanly.
            logs.append_sample(&sample(day2_ts, Some(49.0)), 10_000)?;
            logs.append_disks(
                day2_ts,
                &[DiskSample {
                    mount: PathBuf::from("/"),
                    used_pct: 61.25,
                }],
            )?;
        } // Drop flushes day 2 raw + disks.

        let day1_raw = fs::read_to_string(dir.path().join("raw-2026-11-11.log"))?;
        assert_eq!(day1_raw.lines().count(), 3);
        assert!(day1_raw.contains("48.2 1 10000"));
        assert!(day1_raw.contains(" - 1 10000"));

        let day2_raw = fs::read_to_string(dir.path().join("raw-2026-11-12.log"))?;
        assert_eq!(day2_raw.lines().count(), 2);

        let day2_disks = fs::read_to_string(dir.path().join("disks-2026-11-12.log"))?;
        assert_eq!(day2_disks.lines().count(), 2);
        assert!(day2_disks.contains("/ 61.25"));

        Ok(())
    }

    #[test]
    fn test_daily_logs_immediate_flush_mode() -> Result<()> {
        let dir = TestTempDir::new("savelog_imm")?;
        let mut logs = DailyLogs::new(dir.path().to_path_buf(), true);
        logs.append_sample(&sample(1_794_355_200, Some(42.0)), 5_000)?;

        // Visible on disk immediately without dropping or calling flush().
        let contents = fs::read_to_string(dir.path().join("raw-2026-11-11.log"))?;
        assert_eq!(contents.lines().count(), 2);
        assert!(contents.contains("42.0 1 5000"));

        Ok(())
    }
}
