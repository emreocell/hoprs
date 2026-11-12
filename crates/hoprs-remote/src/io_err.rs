//! Conversion of core errors into `io::Error` for stream adapters.

use std::io;

use hoprs_core::{Error, ErrorKind};

/// Maps a core error kind to an `io::ErrorKind` such that
/// `Error::from(io::Error::new(io_kind_for(kind), ..))` recovers `kind` for
/// `Network`, `NotFound`, `Permission` and `Io`. Every other kind maps to
/// `Other`, which converts back to `Io`.
pub(crate) fn io_kind_for(kind: ErrorKind) -> io::ErrorKind {
    match kind {
        ErrorKind::Network => io::ErrorKind::ConnectionReset,
        ErrorKind::NotFound => io::ErrorKind::NotFound,
        ErrorKind::Permission => io::ErrorKind::PermissionDenied,
        _ => io::ErrorKind::Other,
    }
}

/// Wraps `err` in an `io::Error` whose kind maps back to `err.kind` (see
/// `io_kind_for`) and whose message is `err`'s message.
pub(crate) fn to_io(err: Error) -> io::Error {
    io::Error::new(io_kind_for(err.kind), err)
}
