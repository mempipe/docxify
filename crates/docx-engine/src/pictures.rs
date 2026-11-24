//! Pictures: the media `add_media` adds, `InsertPicture`, and what export
//! adds for the pictures commands make.
//!
//! Added media is named by its bytes alone and kept with the document,
//! outside any command's inverse. Export writes it, with its content type
//! and the relationship to it from each story showing it, while a picture a
//! command made uses it. Undoing the pictures gives back the original
//! parts, and replicas that add the same bytes write the same package.

use std::collections::BTreeSet;
use std::sync::Arc;

use crate::edit::check_text;
use crate::error::EditError;
use crate::export::NewRelationship;
use crate::fnv1a;
use crate::ids;
use crate::ir::{DrawingInfo, MediaRef, NodeId, NodeKind, Placement};
use crate::package::REL_IMAGE;
use crate::xml::{escape_attr, A, R, WP};
use crate::Document;

const PIC: &str = "http://schemas.openxmlformats.org/drawingml/2006/picture";
/// The largest extent `wp:extent` takes (`ST_PositiveCoordinate`).
const MAX_EXTENT: u64 = 27_273_042_316_900;

#[derive(Clone)]
pub(crate) struct AddedMedia {
    pub name: String,
    pub content_type: &'static str,
    pub bytes: Arc<[u8]>,
}

/// A made picture's media, and the story part that shows it.
pub(crate) type Shown<'d> = (&'d str, &'d MediaRef);

/// The extension and content type of a PNG, JPEG or GIF image.
fn sniff(bytes: &[u8]) -> Option<(&'static str, &'static str)> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(("png", "image/png"))
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some(("jpeg", "image/jpeg"))
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some(("gif", "image/gif"))
    } else {
        None
    }
}

/// `to` relative to the directory of `from`, as a relationship target.
fn relative(from: &str, to: &str) -> String {
    let mut dir: Vec<&str> = from.split('/').collect();
    dir.pop();
    let target: Vec<&str> = to.split('/').collect();
    let common = dir
        .iter()
        .zip(&target[..target.len() - 1])
        .take_while(|(a, b)| a == b)
        .count();
    format!(
        "{}{}",
        "../".repeat(dir.len() - common),
        target[common..].join("/")
    )
}

/// The relationship ID of a picture a command makes of `media`. It depends
/// on the media's name alone, as a link's does on its target.
fn picture_id(media: &str) -> String {
    format!("rIdP{:016x}", fnv1a(media.as_bytes()))
}

/// An inline picture's markup. Its namespaces are declared on it, since the
/// story may not declare them.
pub(crate) fn picture_xml(drawing: &str, info: &DrawingInfo) -> String {
    let id = info.id.expect("a made picture has an ID");
    let (cx, cy) = (
        info.width_emu.expect("a width"),
        info.height_emu.expect("a height"),
    );
    let media = &info.media[0];
    let part = media.part.as_deref().expect("a part");
    let file = part.rsplit('/').next().unwrap_or(part);
    let descr = info
        .description
        .as_deref()
        .map_or(String::new(), |d| format!(" descr=\"{}\"", escape_attr(d)));
    format!(
        "<{drawing}><wp:inline xmlns:wp=\"{WP}\" xmlns:a=\"{A}\" xmlns:pic=\"{PIC}\" xmlns:r=\"{R}\" distT=\"0\" distB=\"0\" distL=\"0\" distR=\"0\"><wp:extent cx=\"{cx}\" cy=\"{cy}\"/><wp:docPr id=\"{id}\" name=\"Picture {id}\"{descr}/><wp:cNvGraphicFramePr><a:graphicFrameLocks noChangeAspect=\"1\"/></wp:cNvGraphicFramePr><a:graphic><a:graphicData uri=\"{PIC}\"><pic:pic><pic:nvPicPr><pic:cNvPr id=\"0\" name=\"{}\"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed=\"{}\"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill><pic:spPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"{cx}\" cy=\"{cy}\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></pic:spPr></pic:pic></a:graphicData></a:graphic></wp:inline></{drawing}>",
        escape_attr(file),
        escape_attr(&media.relationship),
    )
}

impl Document {
    /// The part name a picture command uses for these image bytes.
    pub fn media_name(&self, bytes: &[u8]) -> Result<String, EditError> {
        self.media_part(bytes).map(|(name, _)| name)
    }

    fn media_part(&self, bytes: &[u8]) -> Result<(String, &'static str), EditError> {
        let Some((extension, content_type)) = sniff(bytes) else {
            return Err(EditError::Media {
                media: "the bytes".into(),
                reason: "are not a PNG, JPEG or GIF image",
            });
        };
        let main = self.package.main_part();
        let dir = main.rfind('/').map_or("", |i| &main[..=i]);
        let name = format!("{dir}media/image-{:016x}.{extension}", fnv1a(bytes));
        if self.package.actual_name(&name).is_some()
            && !self
                .package
                .read_bytes(&name)
                .is_ok_and(|held| held == bytes)
        {
            return Err(EditError::Media {
                media: name,
                reason: "is already a part with other bytes",
            });
        }
        Ok((name, content_type))
    }

    pub(crate) fn add_media(&mut self, bytes: &[u8]) -> Result<String, EditError> {
        let (name, content_type) = self.media_part(bytes)?;
        if self.package.actual_name(&name).is_some()
            || self.added_media.iter().any(|m| m.name == name)
        {
            return Ok(name);
        }
        self.added_media.push(AddedMedia {
            name: name.clone(),
            content_type,
            bytes: bytes.into(),
        });
        Ok(name)
    }

    /// The bytes `add_media` added as `part`.
    pub(crate) fn added_media_bytes(&self, part: &str) -> Option<&[u8]> {
        let media = self.added_media.iter().find(|m| m.name == part)?;
        Some(&media.bytes)
    }

    /// The content type of media a picture can show: added, or an image
    /// part of the package.
    fn media_type(&self, name: &str) -> Option<String> {
        if let Some(media) = self.added_media.iter().find(|m| m.name == name) {
            return Some(media.content_type.to_string());
        }
        self.package.actual_name(name)?;
        self.package
            .content_type(name)
            .filter(|t| t.starts_with("image/"))
            .map(str::to_string)
    }

    /// What a picture of `media` at this size holds, with the drawing ID it
    /// takes.
    pub(crate) fn picture_info(
        &self,
        media: &str,
        width_emu: u64,
        height_emu: u64,
        description: Option<&str>,
        id: Option<u32>,
    ) -> Result<DrawingInfo, EditError> {
        let refused = |reason| EditError::Media {
            media: media.to_string(),
            reason,
        };
        let content_type = self
            .media_type(media)
            .ok_or_else(|| refused("is not media add_media added or an image in the package"))?;
        if ![width_emu, height_emu]
            .iter()
            .all(|e| (1..=MAX_EXTENT).contains(e))
        {
            return Err(refused(
                "needs a width and a height from 1 to 27273042316900 EMUs",
            ));
        }
        if let Some(description) = description {
            check_text(description)?;
        }
        let id = self
            .drawing_id(id)
            .ok_or_else(|| refused("has no drawing ID left to take"))?;
        Ok(DrawingInfo {
            id: Some(id),
            name: Some(format!("Picture {id}")),
            description: description.map(str::to_string),
            width_emu: Some(width_emu),
            height_emu: Some(height_emu),
            media: vec![MediaRef {
                relationship: picture_id(media),
                part: Some(media.to_string()),
                content_type: Some(content_type),
                external: false,
            }],
            holds: None,
            placement: Placement::Inline,
        })
    }

    /// Refuses media whose relationship ID a relationship of another kind or
    /// to another part already has in the story of `near`. Export adds the
    /// relationship only where the story lacks the ID.
    pub(crate) fn check_picture_id(&self, near: NodeId, media: &str) -> Result<(), EditError> {
        let part = &self.stories[self.n(near).story as usize].part;
        let Some(taken) = self.package.relationship(part, &picture_id(media)) else {
            return Ok(());
        };
        let same = taken.rel_type == REL_IMAGE
            && !taken.external
            && (taken.part.as_deref()).is_some_and(|p| p.eq_ignore_ascii_case(media));
        if same {
            return Ok(());
        }
        Err(EditError::Media {
            media: media.to_string(),
            reason: "takes a relationship ID the story already uses for something else",
        })
    }

    /// The requested ID when Word loads it and no drawing has it, else a free one.
    fn drawing_id(&self, wanted: Option<u32>) -> Option<u32> {
        let made = || {
            self.nodes
                .iter()
                .filter(|n| n.made.is_some())
                .filter_map(|n| match &n.kind {
                    NodeKind::Drawing(info) => Some((n.id, info.id?)),
                    _ => None,
                })
        };
        let free = |id: u32| {
            ids::DRAWING_IDS.contains(&id)
                && !self.source_ids.drawings.contains(&id)
                && !made().any(|(node, made)| made == id && self.attached(node).is_ok())
        };
        if let Some(id) = wanted.filter(|&id| free(id)) {
            return Some(id);
        }
        let drawings = self.source_ids.drawings.iter().copied();
        let used: BTreeSet<u32> = drawings.chain(made().map(|(_, id)| id)).collect();
        ids::next_free(&used, ids::DRAWING_IDS)
    }

    /// The media of each picture a command made that is in a dirty story.
    pub(crate) fn shown(&self) -> Vec<Shown<'_>> {
        let mut out = Vec::new();
        for story in self.stories.iter().filter(|s| self.subtree_dirty(s.root)) {
            for id in self.descendants(story.root) {
                let node = self.n(id);
                if let (NodeKind::Drawing(info), Some(_)) = (&node.kind, &node.made) {
                    out.extend(info.media.iter().map(|m| (story.part.as_str(), m)));
                }
            }
        }
        out
    }

    /// The added media a picture shows, by name, so the order it was added
    /// in does not change what export writes.
    pub(crate) fn media_in_use(&self, shown: &[Shown]) -> Vec<&AddedMedia> {
        let used: BTreeSet<&str> = shown
            .iter()
            .filter_map(|(_, m)| m.part.as_deref())
            .collect();
        let mut media: Vec<&AddedMedia> = self
            .added_media
            .iter()
            .filter(|m| used.contains(m.name.as_str()))
            .collect();
        media.sort_by(|a, b| a.name.cmp(&b.name));
        media
    }

    /// The relationships pictures name that their stories lack, one per ID,
    /// in story order.
    pub(crate) fn picture_relationships(&self, shown: &[Shown]) -> Vec<NewRelationship> {
        let mut out: Vec<NewRelationship> = Vec::new();
        for &(part, media) in shown {
            let Some(target) = media.part.as_deref() else {
                continue;
            };
            let known = self
                .package
                .relationship(part, &media.relationship)
                .is_some()
                || out
                    .iter()
                    .any(|r| r.part == part && r.id == media.relationship);
            if !known {
                out.push(NewRelationship {
                    part: part.to_string(),
                    id: media.relationship.clone(),
                    kind: REL_IMAGE,
                    target: relative(part, target),
                    external: false,
                });
            }
        }
        out
    }
}
