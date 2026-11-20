pub(super) use super::fixtures::*;
pub(super) use docx_engine::editing::SessionError;
use docx_engine::Document;

/// Lets feature tests inspect the current projection between commands.
pub(super) struct EditingDocument(pub docx_engine::editing::Editor);

impl EditingDocument {
    pub fn open(bytes: &[u8]) -> Result<Self, docx_engine::editing::SessionError> {
        docx_engine::editing::Editor::open(bytes).map(Self)
    }
    pub fn run(&mut self, command: docx_engine::Command) -> Result<(), SessionError> {
        self.0.apply(&command)
    }
    #[track_caller]
    pub fn export(&self) -> Result<Vec<u8>, docx_engine::editing::SessionError> {
        let fresh = self.0.export()?;
        let live = self.0.view()?.export()?;
        assert!(live == fresh, "live projection differs from a fresh export");
        Ok(fresh)
    }
}

impl std::ops::Deref for EditingDocument {
    type Target = Document;
    fn deref(&self) -> &Document {
        self.0.view().expect("project the tested editor")
    }
}

pub(super) fn edit(doc: &mut EditingDocument, command: docx_engine::Command) {
    doc.0
        .apply(&command)
        .unwrap_or_else(|err| panic!("{command:?}: {err}"));
}

pub(super) fn assert_editor_undo(name: &str, original: &[u8], doc: &mut EditingDocument) {
    let edited = doc.export().unwrap();
    assert!(doc.0.undo(), "{name}: missing undo");
    assert_eq!(
        stories(doc),
        stories(&Document::open(original).unwrap()),
        "{name}: undo"
    );
    assert_eq!(
        entries(&doc.export().unwrap()),
        entries(original),
        "{name}: undone package"
    );
    assert!(doc.0.redo(), "{name}: missing redo");
    assert!(
        doc.export().unwrap() == edited,
        "{name}: redo differs from the edit"
    );
}
