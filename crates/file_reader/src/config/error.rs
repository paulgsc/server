use thiserror::Error;

#[derive(Debug, Error)]
pub enum PathPartError {
    #[error("Encountered illegal character sequence \"{illegal}\" whilst parsing path segment \"{segment}\"")]
    IllegalCharacter { segment: String, illegal: String },
}
