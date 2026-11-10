//! The one error type of the loader: a message for the user.
//!
//! The Python loader raised LoaderError (a refusal), PayloadError (converted to LoaderError by load) and
//! OSError (Windows calls, NvAPI); every caller handled them the same way, by showing `str(exc)`. Here they are
//! one type that carries that text. Panics are the "unexpected error" path (exit code 70, "Unexpected error:"
//! in the window).

use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    message: String,
}

impl Error {
    pub fn new(message: impl Into<String>) -> Self {
        Error {
            message: message.into(),
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

pub type Result<T, E = Error> = std::result::Result<T, E>;
