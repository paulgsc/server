use thiserror::Error;

#[derive(Debug, Error)]
pub enum PathPartError {
    #[error("Encountered illegal character sequence \"{illegal}\" whilst parsing path segment \"{segment}\"")]
    IllegalCharacter { segment: String, illegal: String },

    #[error("Path \"{path}\" contained empty path segment")]
    EmptySegment {
        /// The source path
        path: String,
    },

    #[error("Error parsing Path \"{path}\": {source}")]
    BadSegment {
        /// The source path
        path: String,
        /// The part containing the error
        source: Box<PathPartError>,
    },

    #[error("Unable to convert path \"{path}\" to URL")]
    InvalidPath {
        /// The source path
        path: std::path::PathBuf,
    },

    /// Error when a path contains non-unicode characters
    #[error("Path \"{path}\" contained non-unicode characters: {source}")]
    NonUnicode {
        /// The source path
        path: String,
        /// The underlying `Utf8Error`
        source: std::str::Utf8Error,
    },

    /// Error when a path doesn't start with the given prefix
    #[error("Path \"{path}\" does not start with prefix \"{prefix}\"")]
    PrefixMismatch {
        /// The source path
        path: String,
        /// The mismatched prefix
        prefix: String,
    },
}
