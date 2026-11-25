//! Collaboration over the core engine's semantic editing state.
//! Text and immutable paired annotations live in named Yrs strands. The
//! core prepares edits and interprets state; this crate owns synchronization,
//! relative positions and local undo. No editing command is inferred from
//! a projected tree, and reading never writes repairs.
use docx_engine::{EditError, ExportError, OpenError};
use sha2::{Digest, Sha256};
mod replica;
pub mod store;
pub use docx_engine::Splice;
pub use replica::{
    build, paragraphs, seed, Char, Inline, Mark, Paragraph, Replica, View, BASIS, FORMAT, MARK,
};
#[derive(Debug, Clone, PartialEq)]
pub enum Redraw {
    All,
    Splices(Vec<Splice>),
}
#[derive(Debug, thiserror::Error)]
pub enum CollabError {
    #[error(transparent)]
    Open(#[from] OpenError),
    #[error(transparent)]
    Edit(#[from] EditError),
    #[error(transparent)]
    Export(#[from] ExportError),
    #[error("invalid Yrs update: {0}")]
    Update(String),
    #[error("the shared state was seeded from base {expected}, not {found}")]
    BaseMismatch { expected: String, found: String },
    #[error("{command} {reason}")]
    Unsupported {
        command: &'static str,
        reason: &'static str,
    },
    #[error("flow {flow}: {reason}")]
    Corrupt { flow: String, reason: String },
    #[error("flow {flow}: its paragraphs could not be built: {reason}")]
    Unbuilt { flow: String, reason: String },
}
impl From<docx_engine::flow::FlowError> for CollabError {
    fn from(error: docx_engine::flow::FlowError) -> Self {
        match error {
            docx_engine::flow::FlowError::Corrupt { flow, reason } => {
                Self::Corrupt { flow, reason }
            }
        }
    }
}

impl From<docx_engine::editing::SessionError> for CollabError {
    fn from(error: docx_engine::editing::SessionError) -> Self {
        use docx_engine::editing::SessionError;
        match error {
            SessionError::Open(e) => Self::Open(e),
            SessionError::Edit(e) => Self::Edit(e),
            SessionError::Export(e) => Self::Export(e),
            SessionError::Flow(e) => e.into(),
            SessionError::Unsupported { command, reason } => Self::Unsupported { command, reason },
            SessionError::Corrupt { flow, reason } => Self::Corrupt { flow, reason },
            SessionError::Unbuilt { flow, reason } => Self::Unbuilt { flow, reason },
        }
    }
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
