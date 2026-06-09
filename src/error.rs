#[derive(Debug, thiserror::Error)]
pub enum TableError {
    #[error("Corrupted table state")]
    CorruptedState,
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Serialization/Decompression error: {0}")]
    Deserialization(String),
    #[error("Operation error: {0}")]
    Operation(String),
    #[error("Generic error: {0}")]
    Generic(String),
}
