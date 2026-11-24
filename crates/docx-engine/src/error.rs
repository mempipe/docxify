use serde::Serialize;

use crate::ir::NodeId;

/// Why a package could not be opened. Opening fails only when the engine
/// cannot model the main document safely; lesser problems become diagnostics.
#[derive(Debug, thiserror::Error)]
pub enum OpenError {
    #[error("not a readable ZIP package: {0}")]
    Zip(String),
    #[error("package is missing {0}")]
    MissingPart(String),
    #[error("package relationships name no main document")]
    NoMainDocument,
    #[error("main document part {0} is missing from the package")]
    MissingMainPart(String),
    #[error("main document has content type {0:?}, not WordprocessingML")]
    NotWordprocessing(Option<String>),
    #[error("{part}: XML is not well-formed: {message}")]
    Xml { part: String, message: String },
    #[error("{part}: unsupported: {reason}")]
    Unsupported { part: String, reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, Serialize)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum EditError {
    #[error("no node {node}")]
    UnknownNode { node: NodeId },
    #[error("{node} is a {kind}, not a text node")]
    NotText { node: NodeId, kind: &'static str },
    #[error("{node} is read-only: {reason}")]
    ReadOnly { node: NodeId, reason: &'static str },
    #[error("range {start}..{end} is outside {node}, which has {len} characters")]
    Range {
        node: NodeId,
        start: usize,
        end: usize,
        len: usize,
    },
    #[error("text cannot contain {found:?}: {reason}")]
    InvalidText { found: char, reason: &'static str },
    #[error("{node} is not in the document; it was removed")]
    Detached { node: NodeId },
    #[error("{node} is a {kind}; expected {expected}")]
    WrongKind {
        node: NodeId,
        kind: &'static str,
        expected: &'static str,
    },
    #[error("{node}: {reason}")]
    Refused { node: NodeId, reason: &'static str },
    #[error("list {num_id} {reason}")]
    List {
        num_id: String,
        reason: &'static str,
    },
    #[error("{media} {reason}")]
    Media { media: String, reason: &'static str },
    #[error("{setting} {reason}")]
    Setting {
        setting: &'static str,
        reason: &'static str,
    },
    #[error("comment {comment} {reason}")]
    Comment {
        comment: String,
        reason: &'static str,
    },
    #[error("fields {reason}")]
    Fields { reason: &'static str },
    #[error("tracked change by {author:?} at {date:?}: {reason}")]
    Track {
        author: String,
        date: String,
        reason: &'static str,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error("could not write the package: {0}")]
    Zip(String),
    #[error(transparent)]
    Read(#[from] OpenError),
}

impl From<zip::result::ZipError> for ExportError {
    fn from(e: zip::result::ZipError) -> Self {
        ExportError::Zip(e.to_string())
    }
}

impl From<std::io::Error> for ExportError {
    fn from(e: std::io::Error) -> Self {
        ExportError::Zip(e.to_string())
    }
}
