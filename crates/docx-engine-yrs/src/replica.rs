//! Yrs adapter for the engine's shared command preparation and builder.
use crate::store::Store;
use crate::CollabError;
use docx_engine::editing::Prepared;
pub use docx_engine::editing::FORMAT;
pub use docx_engine::editing::{Char, Inline, Mark, Paragraph, View, BASIS, MARK};
use docx_engine::{Command, Document, NodeId, Point};
use std::sync::{Arc, Mutex};
const FORMAT_KEY: &str = "format";
const BASE_KEY: &str = "base";

pub fn seed(client: u64, document: &Document, meta: &[(&str, &str)]) -> Result<Store, CollabError> {
    let flow = docx_engine::editing::seed(client, document)?;
    Ok(Store::from_seed(client, meta, &flow))
}
pub fn paragraphs(base: &Document, store: &Store) -> Result<Vec<Paragraph>, CollabError> {
    Ok(docx_engine::editing::paragraphs(base, &store.snapshot()?)?)
}
pub fn build(base: &Document, store: &Store) -> Result<View, CollabError> {
    Ok(docx_engine::editing::build(base, &store.snapshot()?)?)
}
/// One person's replica: an immutable source package and shared semantic
/// state. Commands write that state; the core derives the displayed tree.
pub struct Replica {
    base: Arc<Document>,
    store: Store,
    /// The command state (with its writes drained) and the view it produced.
    projection: Option<Prepared>,
    /// This replica's own changes that `take_update` has not given yet.
    unsent: Arc<Mutex<Vec<Vec<u8>>>>,
    /// Whether the view changed since `take_redraw` last said so.
    redraw: Option<crate::Redraw>,
    dirty: bool,
}

impl Replica {
    /// Starts a shared state from a file, as Yrs client `client`.
    pub fn seed(client: u64, bytes: &[u8]) -> Result<Replica, CollabError> {
        let base = Arc::new(Document::open(bytes)?);
        let meta = [(FORMAT_KEY, FORMAT), (BASE_KEY, &crate::digest(bytes))];
        let store = seed(client, &base, &meta)?;
        Ok(Replica::of(base, store))
    }

    /// Joins the shared state another replica saved. It has to have been
    /// started from the same file: text states runs of that file.
    pub fn join(client: u64, bytes: &[u8], saved: &[u8]) -> Result<Replica, CollabError> {
        let base = Arc::new(Document::open(bytes)?);
        let mut store = Store::from_update(client, saved)?;
        if store.meta(FORMAT_KEY).as_deref() != Some(FORMAT) {
            return Err(CollabError::Unsupported {
                command: "join",
                reason:
                    "requires the current shared format; open this session with its original editor",
            });
        }
        let expected = store.meta(BASE_KEY).unwrap_or_default();
        let found = crate::digest(bytes);
        if expected != found {
            return Err(CollabError::BaseMismatch { expected, found });
        }
        store.scope_strands()?;
        Ok(Replica::of(base, store))
    }

    fn of(base: Arc<Document>, store: Store) -> Replica {
        let unsent = Arc::new(Mutex::new(Vec::new()));
        store.observe_local_updates(unsent.clone());
        Replica {
            base,
            store,
            projection: None,
            unsent,
            redraw: None,
            dirty: true,
        }
    }

    /// Validates and names an image without writing shared state or history.
    pub fn media_name(&self, bytes: &[u8]) -> Result<String, CollabError> {
        Ok(self.base.media_name(bytes)?)
    }

    pub fn save(&self) -> Vec<u8> {
        self.store.save()
    }

    pub fn state_vector(&self) -> Vec<u8> {
        self.store.state_vector()
    }

    pub fn update_since(&self, state_vector: &[u8]) -> Result<Vec<u8>, CollabError> {
        self.store.update_since(state_vector)
    }

    pub fn apply_update(&mut self, update: &[u8]) -> Result<(), CollabError> {
        self.dirty = true;
        self.store.apply_update(update)
    }

    /// Blocks changed since the editor last asked, or a full rebuild.
    pub fn take_redraw(&mut self) -> Result<crate::Redraw, CollabError> {
        self.built()?;
        Ok(self
            .redraw
            .take()
            .unwrap_or(crate::Redraw::Splices(Vec::new())))
    }
    fn record_redraw(&mut self, changes: Option<&[docx_engine::Splice]>) {
        self.redraw = Some(match (self.redraw.take(), changes) {
            (Some(crate::Redraw::All), _) | (_, None) => crate::Redraw::All,
            (Some(prior), Some([])) => prior,
            (Some(crate::Redraw::Splices(prior)), Some(_)) if !prior.is_empty() => {
                // Earlier splices can name blocks that the later edit removed.
                crate::Redraw::All
            }
            (_, Some(changes)) => crate::Redraw::Splices(changes.to_vec()),
        });
    }

    /// What this replica's own changes wrote since this was last called,
    /// undo and redo included, as one update to send.
    pub fn take_update(&mut self) -> Result<Option<Vec<u8>>, CollabError> {
        let mut unsent = self.unsent.lock().unwrap();
        match unsent.len() {
            0 => Ok(None),
            1 => Ok(unsent.pop()),
            _ => {
                let merged = yrs::merge_updates_v1(std::mem::take(&mut *unsent));
                merged
                    .map(Some)
                    .map_err(|e| CollabError::Update(e.to_string()))
            }
        }
    }

    fn built(&mut self) -> Result<&Prepared, CollabError> {
        if self.dirty {
            let state = self.store.snapshot()?;
            let previous = self.projection.as_ref().map(|p| &p.view);
            let view = docx_engine::editing::rebuild(&self.base, &state, previous)?;
            if previous.is_some() {
                self.record_redraw(view.changes.as_deref());
            }
            self.projection = Some(Prepared { state, view });
            self.dirty = false;
        }
        Ok(self.projection.as_ref().expect("built"))
    }

    /// The document as the shared state reads now.
    pub fn view(&mut self) -> Result<&Document, CollabError> {
        Ok(&self.built()?.view.document)
    }

    pub fn sticky(&mut self, point: &Point) -> Result<Option<(String, Vec<u8>)>, CollabError> {
        let Ok(at) = self.built()?.view.position(point) else {
            return Ok(None);
        };
        let key = self
            .store
            .meta(BASE_KEY)
            .expect("base metadata was validated");
        let state = self
            .projection
            .as_ref()
            .expect("built")
            .view
            .position_state();
        Ok(self.store.sticky(state, at)?.map(|bytes| (key, bytes)))
    }
    pub fn resolve(
        &mut self,
        key: &str,
        token: &[u8],
    ) -> Result<Option<(NodeId, Point)>, CollabError> {
        if self.store.meta(BASE_KEY).as_deref() != Some(key) {
            return Ok(None);
        }
        self.built()?;
        let view = &self.projection.as_ref().expect("built").view;
        let Some(at) = self.store.resolve(view.position_state(), token)? else {
            return Ok(None);
        };
        Ok(view.point_at(at))
    }

    pub fn resolutions(&mut self) -> Result<&[docx_engine::editing::Resolution], CollabError> {
        Ok(&self.built()?.view.resolutions)
    }

    pub fn export(&mut self) -> Result<Vec<u8>, CollabError> {
        Ok(docx_engine::editing::export(
            &self.base,
            &self.store.snapshot()?,
        )?)
    }

    /// Writes what a command asks for to the flow, as one change. The
    /// command names nodes of the current view. A command that is refused
    /// writes nothing.
    pub fn apply(&mut self, command: &Command) -> Result<(), CollabError> {
        self.apply_all(std::slice::from_ref(command))
    }

    /// Writes several commands as one change, each naming nodes of the
    /// view the one before it left. Prepare in isolation and check the final
    /// document before committing: a refusal changes neither shared state
    /// nor undo/redo history, and sends nothing to other replicas.
    pub fn apply_all(&mut self, commands: &[Command]) -> Result<(), CollabError> {
        self.apply_all_with_media(commands, &[])
    }

    /// Prepares images and commands privately, then commits both together.
    /// Accepted image bytes survive undo so redo and other references work.
    pub fn apply_all_with_media(
        &mut self,
        commands: &[Command],
        media: &[&[u8]],
    ) -> Result<(), CollabError> {
        let base = self.base.clone();
        let current = self.built()?;
        let mut prepared = docx_engine::editing::prepare(
            base,
            current.state.clone(),
            Some(current.view.clone()),
            commands,
            media,
        )?;
        let predicted = prepared.state.next_identity().1;
        let writes = prepared.state.take_writes();
        if writes.is_empty() {
            // The current projection and any pending redraw still describe the document.
            return Ok(());
        }
        self.store.commit_prepared(writes);
        assert_eq!(
            self.store.local_clock(),
            predicted,
            "prepared writes diverged from the Yrs clock"
        );
        self.store.step();
        if commands.len() == 1 {
            self.record_redraw(prepared.view.changes.as_deref());
        } else {
            // Each command can replace nodes named by an earlier splice.
            self.record_redraw(None);
        }
        self.projection = Some(prepared);
        Ok(())
    }

    /// A standalone editor needs no transport updates.
    pub fn stop_local_updates(&self) {
        self.store.stop_local_updates();
    }

    pub fn comment_identities(
        &mut self,
    ) -> Result<std::collections::HashMap<String, String>, CollabError> {
        Ok(self.built()?.view.comment_identities())
    }

    /// Takes back this replica's last change.
    pub fn undo(&mut self) -> Result<bool, CollabError> {
        let undone = self.store.undo();
        if undone {
            self.dirty = true;
            self.store.step();
        }
        Ok(undone)
    }

    /// Puts back the change this replica last took back.
    pub fn redo(&mut self) -> Result<bool, CollabError> {
        let redone = self.store.redo();
        if redone {
            self.dirty = true;
            self.store.step();
        }
        Ok(redone)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Compare the raw positions used to prepare the next write, including
    /// invisible range boundaries. Two states can export identical text while
    /// addressing different Yrs items.
    fn assert_fresh(replica: &mut Replica) {
        let fresh = replica.store.snapshot().unwrap();
        let state = &replica.built().unwrap().state;
        assert_eq!(state.next_identity(), fresh.next_identity());
        assert!(state.clone().take_writes().is_empty());
        assert_eq!(state.facts(), fresh.facts());
        assert_eq!(state.strands().len(), fresh.strands().len());
        for (cached, stored) in state.strands().iter().zip(fresh.strands()) {
            assert!(cached.declared);
            assert_eq!(cached.key, stored.key);
            assert_eq!(cached.order, stored.order);
            assert_eq!(cached.prefix, stored.prefix);
            assert_eq!(cached.flow.slots(), stored.flow.slots());
        }
    }

    #[test]
    fn next_command_uses_the_committed_state_after_edits_refusals_sync_and_history() {
        let bytes = include_bytes!("../../../fixtures/generated/table.docx");
        let mut replica = Replica::seed(1, bytes).unwrap();
        let mut peer = Replica::join(2, bytes, &replica.save()).unwrap();
        assert_fresh(&mut replica);
        let doc = replica.view().unwrap();
        let row = doc
            .stories()
            .flat_map(|s| doc.descendants(s.id))
            .find(|id| matches!(doc.node(*id).unwrap().kind, docx_engine::NodeKind::Row(_)))
            .unwrap();
        let table = doc.node(row).unwrap().parent.unwrap();
        replica
            .apply(&Command::InsertRow {
                row,
                below: true,
                like: None,
            })
            .unwrap();
        assert_fresh(&mut replica);
        let doc = replica.view().unwrap();
        let inserted = doc.node(table).unwrap().children[1];
        let cell = doc.node(inserted).unwrap().children[0];
        let p = doc.node(cell).unwrap().children[0];
        let type_in_cell = Command::InsertText {
            at: Point::End { end: p },
            text: "🦀".into(),
        };
        replica.apply(&type_in_cell).unwrap();
        assert_fresh(&mut replica);
        let invalid = Command::InsertText {
            at: Point::End { end: p },
            text: "\t".into(),
        };
        assert!(replica.apply_all(&[type_in_cell.clone(), invalid]).is_err());
        assert_fresh(&mut replica);
        let first = peer.view().unwrap().paragraphs()[0];
        peer.apply(&Command::InsertText {
            at: Point::Start { start: first },
            text: "peer".into(),
        })
        .unwrap();
        peer.apply(&Command::SetUpdateFields { on: true }).unwrap();
        replica
            .apply_update(&peer.take_update().unwrap().unwrap())
            .unwrap();
        assert_fresh(&mut replica);
        replica.apply(&type_in_cell).unwrap();
        assert_fresh(&mut replica);
        assert!(replica.undo().unwrap());
        assert_fresh(&mut replica);
        assert!(replica.redo().unwrap());
        assert_fresh(&mut replica);
    }
}
