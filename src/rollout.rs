use std::fs;
use std::io;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use time::OffsetDateTime;
use time::macros::format_description;

use crate::ArchiveType;

pub fn rotate(
    log_folder: &PathBuf,
    archive_type: &ArchiveType,
) -> Result<(), Box<dyn std::error::Error>> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    Ok(())
}
