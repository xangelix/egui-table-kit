#[derive(Debug, thiserror::Error)]
pub enum TableError {
    /// Raised when internal table indices or states get corrupted or out of sync.
    #[error("Corrupted table state")]
    CorruptedState,

    /// Standard Input/Output errors.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// Failures in data decompression or deserialization.
    #[error("Serialization/Decompression error: {0}")]
    Deserialization(String),

    /// Failures originating from background asynchronous operations.
    #[error("Operation error: {0}")]
    Operation(String),

    /// General generic error wrapper.
    #[error("Generic error: {0}")]
    Generic(String),
}
