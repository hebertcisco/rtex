//! Errors that stop a compilation before or after TeX runs.

use std::io;
use std::path::PathBuf;

/// Why a file could not be compiled.
#[derive(Debug, thiserror::Error)]
pub enum CompileError {
    /// The input file does not exist or is not a file.
    #[error("input file not found: {}", .0.display())]
    InputNotFound(PathBuf),
    /// The input path has no file name TeX can read.
    #[error("input path has no usable file name: {}", .0.display())]
    InvalidInput(PathBuf),
    /// The input could not be read.
    #[error("cannot read {}: {source}", path.display())]
    Read {
        /// The file.
        path: PathBuf,
        /// The underlying error.
        #[source]
        source: io::Error,
    },
    /// A format image could not be built.
    #[error("cannot build a format: {message}")]
    Format {
        /// What TeX said.
        message: String,
    },
    /// An output file could not be written.
    #[error("cannot write {}: {source}", path.display())]
    Write {
        /// The file.
        path: PathBuf,
        /// The underlying error.
        #[source]
        source: io::Error,
    },
}
