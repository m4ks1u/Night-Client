use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CoreError {
    #[error("invalid Maven coordinate `{0}`")]
    InvalidCoordinate(String),
    #[error("unsafe relative path `{0}`")]
    UnsafePath(String),
    #[error("version `{version}` is missing required field `{field}`")]
    MissingField {
        version: String,
        field: &'static str,
    },
    #[error("version `{child}` inherits from `{expected}` but parent `{actual}` was supplied")]
    ParentMismatch {
        child: String,
        expected: String,
        actual: String,
    },
}
