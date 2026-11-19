use std::fmt;
use std::io;
use std::net::AddrParseError;
use std::num::{ParseFloatError, ParseIntError};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    Nix(nix::errno::Errno),
    TimeRange(time::error::ComponentRange),
    TimeFormat(time::error::Format),
    Cli(lexopt::Error),
    Signal(ctrlc::Error),
    Parse(String),
    InvalidArg(String),
    Server(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "I/O error: {e}"),
            Self::Nix(e) => write!(f, "syscall error: {e}"),
            Self::TimeRange(e) => write!(f, "timestamp out of range: {e}"),
            Self::TimeFormat(e) => write!(f, "time format error: {e}"),
            Self::Cli(e) => write!(f, "argument error: {e}"),
            Self::Signal(e) => write!(f, "signal handler error: {e}"),
            Self::Parse(msg) => write!(f, "parse error: {msg}"),
            Self::InvalidArg(msg) => write!(f, "invalid argument: {msg}"),
            Self::Server(msg) => write!(f, "server error: {msg}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::Nix(e) => Some(e),
            Self::TimeRange(e) => Some(e),
            Self::TimeFormat(e) => Some(e),
            Self::Cli(e) => Some(e),
            Self::Signal(e) => Some(e),
            Self::Parse(_) | Self::InvalidArg(_) | Self::Server(_) => None,
        }
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<nix::errno::Errno> for Error {
    fn from(e: nix::errno::Errno) -> Self {
        Self::Nix(e)
    }
}

impl From<time::error::ComponentRange> for Error {
    fn from(e: time::error::ComponentRange) -> Self {
        Self::TimeRange(e)
    }
}

impl From<time::error::Format> for Error {
    fn from(e: time::error::Format) -> Self {
        Self::TimeFormat(e)
    }
}

impl From<lexopt::Error> for Error {
    fn from(e: lexopt::Error) -> Self {
        Self::Cli(e)
    }
}

impl From<ctrlc::Error> for Error {
    fn from(e: ctrlc::Error) -> Self {
        Self::Signal(e)
    }
}

impl From<AddrParseError> for Error {
    fn from(e: AddrParseError) -> Self {
        Self::InvalidArg(e.to_string())
    }
}

impl From<ParseIntError> for Error {
    fn from(e: ParseIntError) -> Self {
        Self::InvalidArg(e.to_string())
    }
}

impl From<ParseFloatError> for Error {
    fn from(e: ParseFloatError) -> Self {
        Self::InvalidArg(e.to_string())
    }
}

impl From<&'static str> for Error {
    fn from(msg: &'static str) -> Self {
        Self::Parse(msg.to_string())
    }
}

impl From<String> for Error {
    fn from(msg: String) -> Self {
        Self::Parse(msg)
    }
}
