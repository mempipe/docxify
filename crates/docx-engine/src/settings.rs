//! `SetUpdateFields`: whether Word updates the document's fields when it
//! opens the file, the settings part's `w:updateFields`.
//!
//! Export rewrites only that element, and only when the command left it
//! other than the file had it. A document without a settings part gets one
//! that holds just the flag.

use crate::error::ExportError;
use crate::export::{Additions, DECLARATION};
use crate::xml::{self, W};
use crate::Document;

pub(crate) const CT_SETTINGS: &str =
    "application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml";

/// The settings the schema puts after `w:updateFields`. Elements from other
/// namespaces, such as `m:mathPr` and Word's `w14:docId`, come after it too.
const AFTER: [&str; 21] = [
    "hdrShapeDefaults",
    "footnotePr",
    "endnotePr",
    "compat",
    "docVars",
    "rsids",
    "mathPr",
    "attachedSchema",
    "themeFontLang",
    "clrSchemeMapping",
    "doNotIncludeSubdocsInStats",
    "doNotAutoCompressPictures",
    "forceUpgrade",
    "captions",
    "readModeInkLockDown",
    "smartTagType",
    "schemaLibrary",
    "shapeDefaults",
    "doNotEmbedSmartTags",
    "decimalSymbol",
    "listSeparator",
];

impl Document {
    /// The settings part export writes: the document's, or a new one when
    /// it has none and the flag is on. None while the flag is as the file
    /// had it.
    pub(crate) fn settings_part(&self) -> Option<String> {
        if self.settings.update_fields == self.opened_update_fields {
            return None;
        }
        if let Some(part) = &self.settings_part {
            return Some(part.clone());
        }
        let main = self.package.main_part();
        let dir = main.rfind('/').map_or("", |i| &main[..=i]);
        let part = (0..)
            .map(|n| match n {
                0 => format!("{dir}settings.xml"),
                n => format!("{dir}settings{n}.xml"),
            })
            .find(|p| self.package.actual_name(p).is_none())
            .expect("a free part name");
        Some(part)
    }

    /// Writes the settings part with the flag into `out`, when it changed.
    pub(crate) fn settings_parts(
        &self,
        part: String,
        out: &mut Additions,
    ) -> Result<(), ExportError> {
        let on = self.settings.update_fields;
        let Some(existing) = &self.settings_part else {
            out.added.push((
                part,
                format!(
                    "{DECLARATION}<w:settings xmlns:w=\"{W}\"><w:updateFields w:val=\"true\"/></w:settings>"
                ),
            ));
            return Ok(());
        };
        let text = self.edit_root_ranges(existing, |root, _, close| {
            let (p, declare) = match root.lookup_prefix(W) {
                Some(prefix) => (format!("{prefix}:"), String::new()),
                None => ("w:".to_string(), format!(" xmlns:w=\"{W}\"")),
            };
            let markup = match on {
                true => format!("<{p}updateFields{declare} {p}val=\"true\"/>"),
                false => String::new(),
            };
            let current = xml::elements(root).find(|c| xml::is(*c, W, "updateFields"));
            if let Some(current) = current {
                return vec![(current.range(), markup)];
            }
            let at = xml::elements(root)
                .find(|c| {
                    c.tag_name().namespace() != Some(W) || AFTER.contains(&c.tag_name().name())
                })
                .map_or(close, |c| c.range().start);
            vec![(at..at, markup)]
        })?;
        out.replaced.insert(existing.to_ascii_lowercase(), text);
        Ok(())
    }
}
