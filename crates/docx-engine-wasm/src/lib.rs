//! WebAssembly bindings: the same engine, exposed to JavaScript.
//!
//! Structured results cross the boundary as JSON strings. Text offsets are
//! Unicode scalar values (code points), as in the Rust API, not UTF-16 units.

use docx_engine::{validate_package, Command, Document, NodeId, Point};
use docx_engine_yrs::Replica;
use wasm_bindgen::prelude::*;

#[wasm_bindgen(js_name = formatNumber)]
pub fn format_number(n: u32, format: &str) -> String {
    docx_engine::format_number(n, format)
}

fn js_error(e: impl std::fmt::Display) -> JsError {
    JsError::new(&e.to_string())
}

fn inspect_node(doc: &Document, node: &str) -> Result<String, JsError> {
    let id: NodeId = node.parse().map_err(js_error)?;
    let view = doc
        .inspect_node(id)
        .ok_or_else(|| JsError::new(&format!("{id} is not in the document")))?;
    Ok(serde_json::to_string(&view).expect("inspection serializes"))
}

fn text(doc: &Document, node: &str) -> Result<Option<String>, JsError> {
    let id: NodeId = node.parse().map_err(js_error)?;
    Ok(doc.text(id))
}

#[wasm_bindgen]
pub struct DocxDocument {
    inner: Document,
}

#[wasm_bindgen]
impl DocxDocument {
    pub fn open(bytes: &[u8]) -> Result<DocxDocument, JsError> {
        Document::open(bytes)
            .map(|inner| DocxDocument { inner })
            .map_err(js_error)
    }

    /// The inspection tree as JSON (`docx-engine.inspect/2`).
    pub fn inspect(&self) -> String {
        serde_json::to_string(&self.inner.inspect()).expect("inspection serializes")
    }

    /// A paragraph's or text node's text.
    pub fn text(&self, node: &str) -> Result<Option<String>, JsError> {
        text(&self.inner, node)
    }
}

/// One participant's semantic editing state, synchronized through Yrs.
/// View IDs belong to the current local projection; relative positions
/// identify a selection across replicas and projection changes.
#[wasm_bindgen]
pub struct DocxReplica {
    inner: Replica,
}

#[wasm_bindgen]
impl DocxReplica {
    pub fn seed(client_id: u32, base: &[u8]) -> Result<DocxReplica, JsError> {
        let inner = Replica::seed(client_id.into(), base).map_err(js_error)?;
        Ok(DocxReplica { inner })
    }

    pub fn join(client_id: u32, base: &[u8], update: &[u8]) -> Result<DocxReplica, JsError> {
        let inner = Replica::join(client_id.into(), base, update).map_err(js_error)?;
        Ok(DocxReplica { inner })
    }

    #[wasm_bindgen(js_name = commentIdentities)]
    pub fn comment_identities(&mut self) -> Result<String, JsError> {
        Ok(serde_json::to_string(&self.inner.comment_identities().map_err(js_error)?).unwrap())
    }

    #[wasm_bindgen(js_name = stopLocalUpdates)]
    pub fn stop_local_updates(&self) {
        self.inner.stop_local_updates()
    }

    #[wasm_bindgen(js_name = mediaName)]
    pub fn media_name(&self, bytes: &[u8]) -> Result<String, JsError> {
        self.inner.media_name(bytes).map_err(js_error)
    }

    pub fn snapshot(&self) -> Vec<u8> {
        self.inner.save()
    }

    #[wasm_bindgen(js_name = stateVector)]
    pub fn state_vector(&self) -> Vec<u8> {
        self.inner.state_vector()
    }

    #[wasm_bindgen(js_name = updateSince)]
    pub fn update_since(&self, state_vector: &[u8]) -> Result<Vec<u8>, JsError> {
        self.inner.update_since(state_vector).map_err(js_error)
    }

    #[wasm_bindgen(js_name = takeUpdate)]
    pub fn take_update(&mut self) -> Result<Option<Vec<u8>>, JsError> {
        self.inner.take_update().map_err(js_error)
    }

    #[wasm_bindgen(js_name = applyUpdate)]
    pub fn apply_update(&mut self, update: &[u8]) -> Result<(), JsError> {
        self.inner.apply_update(update).map_err(js_error)
    }

    pub fn apply(&mut self, commands: &str, media: Option<Vec<u8>>) -> Result<(), JsError> {
        let commands: Vec<Command> = serde_json::from_str(commands).map_err(js_error)?;
        self.inner
            .apply_all_with_media(&commands, &media.as_deref().into_iter().collect::<Vec<_>>())
            .map_err(js_error)
    }

    pub fn undo(&mut self) -> Result<bool, JsError> {
        self.inner.undo().map_err(js_error)
    }

    pub fn redo(&mut self) -> Result<bool, JsError> {
        self.inner.redo().map_err(js_error)
    }

    /// `"all"` for a full redraw, otherwise paragraph splices (`[]` if unchanged).
    #[wasm_bindgen(js_name = takeRedraw)]
    pub fn take_redraw(&mut self) -> Result<String, JsError> {
        Ok(match self.inner.take_redraw().map_err(js_error)? {
            docx_engine_yrs::Redraw::All => "\"all\"".into(),
            docx_engine_yrs::Redraw::Splices(splices) => {
                serde_json::to_string(&splices).expect("splices serialize")
            }
        })
    }

    /// A caret at `point` (JSON) as an opaque token that stays beside the
    /// same content while others edit, for showing it to them; `undefined`
    /// if the point is not in the view.
    pub fn sticky(&mut self, point: &str) -> Result<Option<String>, JsError> {
        let point: Point = serde_json::from_str(point).map_err(js_error)?;
        Ok(self
            .inner
            .sticky(&point)
            .map_err(js_error)?
            .map(|(key, bytes)| {
                let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
                format!("{key}:{hex}")
            }))
    }

    /// Where a token from [`sticky`](DocxReplica::sticky) is now, as JSON:
    /// `{paragraph, point}`; `undefined` if it is not in the view.
    pub fn resolve(&mut self, token: &str) -> Result<Option<String>, JsError> {
        let Some((key, hex)) = token.rsplit_once(':') else {
            return Ok(None);
        };
        let Some(bytes) = unhex(hex) else {
            return Ok(None);
        };
        Ok(self
            .inner
            .resolve(key, &bytes)
            .map_err(js_error)?
            .map(|(paragraph, point)| {
                serde_json::json!({ "paragraph": paragraph.to_string(), "point": point })
                    .to_string()
            }))
    }

    pub fn inspect(&mut self) -> Result<String, JsError> {
        let inspection = self.inner.view().map_err(js_error)?.inspect();
        Ok(serde_json::to_string(&inspection).expect("inspection serializes"))
    }

    #[wasm_bindgen(js_name = inspectNode)]
    pub fn inspect_node(&mut self, node: &str) -> Result<String, JsError> {
        inspect_node(self.inner.view().map_err(js_error)?, node)
    }

    #[wasm_bindgen(js_name = inspectChildren)]
    pub fn inspect_children(
        &mut self,
        parent: &str,
        start: usize,
        end: usize,
    ) -> Result<String, JsError> {
        let id = parent.parse().map_err(js_error)?;
        let children = self
            .inner
            .view()
            .map_err(js_error)?
            .inspect_children(id, start..end)
            .ok_or_else(|| JsError::new(&format!("no children {start}..{end} of {id}")))?;
        Ok(serde_json::to_string(&children).expect("inspection serializes"))
    }

    pub fn settings(&mut self) -> Result<String, JsError> {
        let settings = self.inner.view().map_err(js_error)?.settings();
        Ok(serde_json::to_string(settings).expect("settings serialize"))
    }

    pub fn text(&mut self, node: &str) -> Result<Option<String>, JsError> {
        text(self.inner.view().map_err(js_error)?, node)
    }

    pub fn part(&mut self, name: &str) -> Result<Option<Vec<u8>>, JsError> {
        Ok(self.inner.view().map_err(js_error)?.part_bytes(name))
    }

    pub fn resolutions(&mut self) -> Result<String, JsError> {
        Ok(
            serde_json::to_string(self.inner.resolutions().map_err(js_error)?)
                .expect("resolutions serialize"),
        )
    }

    pub fn export(&mut self) -> Result<Vec<u8>, JsError> {
        self.inner.export().map_err(js_error)
    }
}

fn unhex(hex: &str) -> Option<Vec<u8>> {
    if !hex.len().is_multiple_of(2) {
        return None;
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok())
        .collect()
}

/// Validate a package; returns the report as JSON.
#[wasm_bindgen]
pub fn validate(bytes: &[u8]) -> String {
    serde_json::to_string(&validate_package(bytes)).expect("report serializes")
}

/// Format written by this build of the replica.
#[wasm_bindgen(js_name = sharedFormat)]
pub fn shared_format() -> String {
    docx_engine_yrs::FORMAT.to_owned()
}
