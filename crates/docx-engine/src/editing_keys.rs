//! Names encoded in the shared editing format.
/// An item that starts a paragraph.
pub const MARK: &str = "p";
/// What text came with: a run of the file, or a paragraph of the file for
/// text typed where there was none.
pub const BASIS: &str = "basis";
pub(super) const INLINE: &str = "inline";
pub(super) const SCOPE: &str = "scope";
pub(super) const NEW_INLINE: &str = "new_inline";
pub(super) const INSERTION: &str = "insertion";
/// The unique generation minted by an insertion command.
pub const BIRTH: &str = "birth";
pub(super) const DELETION: &str = "deletion";

pub(super) const STYLE: &str = "style";
pub(super) const NUMBERING: &str = "numbering";
pub(super) const LINK: &str = "link";
/// A stated empty target removes the source link. Absence keeps it.
pub(super) const UNLINKED: &str = "";

/// Shared editing state understood by the reader and writer.
pub const FORMAT: &str = "docx-engine.yrs/9";
pub(super) const MEDIA: &str = "media/";
pub(super) const CONTROL: &str = "control/";
pub(super) const FIELD: &str = "field/";
pub(super) const UPDATE_FIELDS: &str = "settings/updateFields";
pub const CELL: &str = "cell/";
pub(super) const CELL_BASIS: &str = "cell_basis/";
pub(super) const STRUCTURE_PAYLOAD: &str = "structure/payload/";
pub(super) const STRUCTURE_ACTIVE: &str = "structure/active/";
pub(super) const COMMENT_PAYLOAD: &str = "comment/";
pub(super) const COMMENT: &str = "comment";

/// Explicitly no source properties; distinct from an absent basis value.
pub(super) const BARE: &str = "";
pub(super) const BOLD: &str = "b";
pub(super) const ITALIC: &str = "i";
pub(super) const UNDERLINE: &str = "u";
