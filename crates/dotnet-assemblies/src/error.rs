use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq)]
pub enum AssemblyLoadError {
    #[error("File not found: {0}")]
    FileNotFound(String),
    #[error("Invalid format: {0}")]
    InvalidFormat(String),
    #[error("IO error: {0}")]
    Io(String),
}
