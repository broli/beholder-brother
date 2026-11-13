use std::fs;
use std::io;
use time::OffsetDateTime;
use time::macros::format_description;

#[derive(Default)]
pub enum ArchiveType {
    Plain,
    #[default]
    Gzip,
    None,
}

pub struct RotationStatus {
    last_rotated_date: Option<time::Date>,
    days_since_last_rotation: i64,
}

impl ArchiveType {
    pub fn from_extension(ext: &str) -> Self {
        match ext {
            "gz" => Self::Gzip,
            "txt" | "" => Self::Plain,
            _ => Self::None,
        }
    }
}

impl ArchiveType {
    pub fn extension(&self) -> &str {
        match self {
            Self::Gzip => "gz",
            Self::Plain => "txt",
            Self::None => "",
        }
    }
}

pub fn rotate() {
    //TODO
}
