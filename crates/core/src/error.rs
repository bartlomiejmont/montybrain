use std::path::PathBuf;

/// Errors returned by vault operations.
///
/// Parsing problems inside individual files are *not* errors: they are reported as
/// [`crate::LoadWarning`]s in the snapshot. Errors are reserved for I/O failures,
/// missing entities and invalid input to mutations.
///
/// `Error` serializes as its display string, so it can be returned directly from a
/// Tauri command or embedded in an MCP error payload.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("I/O error at {}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("vault directory does not exist: {}", .0.display())]
    VaultNotFound(PathBuf),
    #[error("cannot determine the vault directory: set MONTYBRAIN_DIR or HOME")]
    NoVaultDir,
    #[error("project not found: {0}")]
    ProjectNotFound(String),
    #[error("topic not found: {project}/{topic}")]
    TopicNotFound { project: String, topic: String },
    #[error("task not found: ^{0}")]
    TaskNotFound(String),
    #[error("invalid input: {0}")]
    InvalidInput(String),
    /// A file needed by a mutation cannot be edited safely (e.g. it is not valid UTF-8).
    #[error("cannot edit {}: {message}", path.display())]
    Unparseable { path: PathBuf, message: String },
}

impl Error {
    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Error::Io {
            path: path.into(),
            source,
        }
    }

    pub(crate) fn invalid(msg: impl Into<String>) -> Self {
        Error::InvalidInput(msg.into())
    }
}

impl serde::Serialize for Error {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
