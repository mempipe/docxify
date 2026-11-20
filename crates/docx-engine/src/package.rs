//! The OPC package: ZIP entries, content types and relationships.
//!
//! The original ZIP bytes stay in memory so untouched entries can be copied
//! to the export without being decompressed or recompressed. Only the parts
//! the engine models are decoded.

use std::collections::{BTreeMap, HashMap};
use std::io::{Cursor, Read};
use std::sync::Arc;

use serde::Serialize;
use zip::ZipArchive;

use crate::error::OpenError;
use crate::xml::{self, CONTENT_TYPES, PKG_RELS};

pub const REL_OFFICE_DOCUMENT: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument";
pub const REL_OFFICE_DOCUMENT_STRICT: &str =
    "http://purl.oclc.org/ooxml/officeDocument/relationships/officeDocument";
pub const REL_STYLES: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles";
pub const REL_NUMBERING: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering";
pub const REL_IMAGE: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image";
pub const REL_HYPERLINK: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink";
pub const REL_HEADER: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/header";
pub const REL_FOOTER: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/footer";
pub const REL_FOOTNOTES: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/footnotes";
pub const REL_ENDNOTES: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/endnotes";
pub const REL_COMMENTS: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments";
pub const REL_SETTINGS: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/settings";
pub const REL_THEME: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme";

const MAIN_CONTENT_TYPES: [&str; 4] = [
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml",
    "application/vnd.openxmlformats-officedocument.wordprocessingml.template.main+xml",
    "application/vnd.ms-word.document.macroEnabled.main+xml",
    "application/vnd.ms-word.template.macroEnabledTemplate.main+xml",
];

#[derive(Debug, Clone, Serialize)]
pub struct Relationship {
    pub id: String,
    #[serde(rename = "type")]
    pub rel_type: String,
    pub target: String,
    pub external: bool,
    /// The part an internal target resolves to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub part: Option<String>,
}

#[derive(Debug, Default, Clone)]
pub struct ContentTypes {
    defaults: HashMap<String, String>,
    overrides: HashMap<String, String>,
}

impl ContentTypes {
    pub fn parse(text: &str) -> Result<Self, OpenError> {
        let doc = xml::parse("[Content_Types].xml", text)?;
        let mut types = ContentTypes::default();
        for el in xml::elements(doc.root_element()) {
            if xml::is(el, CONTENT_TYPES, "Default") {
                if let (Some(ext), Some(ct)) =
                    (el.attribute("Extension"), el.attribute("ContentType"))
                {
                    types
                        .defaults
                        .insert(ext.to_ascii_lowercase(), ct.to_string());
                }
            } else if xml::is(el, CONTENT_TYPES, "Override") {
                if let (Some(name), Some(ct)) =
                    (el.attribute("PartName"), el.attribute("ContentType"))
                {
                    let name = name.trim_start_matches('/').to_ascii_lowercase();
                    types.overrides.insert(name, ct.to_string());
                }
            }
        }
        Ok(types)
    }

    pub fn get(&self, part: &str) -> Option<&str> {
        let key = part.to_ascii_lowercase();
        if let Some(ct) = self.overrides.get(&key) {
            return Some(ct);
        }
        let ext = key.rsplit_once('.').map(|(_, ext)| ext)?;
        self.defaults.get(ext).map(String::as_str)
    }
}

pub fn parse_rels(
    source_part: &str,
    rels_part: &str,
    text: &str,
) -> Result<Vec<Relationship>, OpenError> {
    let doc = xml::parse(rels_part, text)?;
    let mut rels = Vec::new();
    for el in xml::elements(doc.root_element()).filter(|e| xml::is(*e, PKG_RELS, "Relationship")) {
        let (Some(id), Some(rel_type), Some(target)) = (
            el.attribute("Id"),
            el.attribute("Type"),
            el.attribute("Target"),
        ) else {
            continue;
        };
        let external = el.attribute("TargetMode") == Some("External");
        rels.push(Relationship {
            id: id.to_string(),
            rel_type: rel_type.to_string(),
            target: target.to_string(),
            external,
            part: (!external).then(|| resolve_target(source_part, target)),
        });
    }
    Ok(rels)
}

/// The part a `.rels` part describes, or `None` if the name is not a rels part.
pub fn source_part_for(rels_part: &str) -> Option<String> {
    let (dir, file) = rels_part.rsplit_once("_rels/")?;
    let name = file.strip_suffix(".rels")?;
    Some(format!("{dir}{name}"))
}

/// Resolve a relationship target against its source part, per OPC: relative
/// to the source part's directory, or absolute from the package root.
pub fn resolve_target(source_part: &str, target: &str) -> String {
    let target = target.split('#').next().unwrap_or(target);
    let joined = match target.strip_prefix('/') {
        Some(absolute) => absolute.to_string(),
        None => match source_part.rsplit_once('/') {
            Some((dir, _)) => format!("{dir}/{target}"),
            None => target.to_string(),
        },
    };
    let mut segments: Vec<&str> = Vec::new();
    for segment in joined.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            s => segments.push(s),
        }
    }
    percent_decode(&segments.join("/"))
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = |b: u8| (b as char).to_digit(16);
            if let (Some(hi), Some(lo)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push((hi * 16 + lo) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}

#[derive(Clone)]
pub struct Package {
    zip: Arc<[u8]>,
    entries: Vec<String>,
    index: HashMap<String, usize>,
    content_types: ContentTypes,
    rels: BTreeMap<String, Vec<Relationship>>,
    main_part: String,
}

impl Package {
    pub fn open(bytes: &[u8]) -> Result<Self, OpenError> {
        let zip: Arc<[u8]> = Arc::from(bytes);
        let mut archive =
            ZipArchive::new(Cursor::new(&zip[..])).map_err(|e| OpenError::Zip(e.to_string()))?;
        let mut entries = Vec::with_capacity(archive.len());
        let mut index = HashMap::with_capacity(archive.len());
        for i in 0..archive.len() {
            let name = archive
                .name_for_index(i)
                .ok_or_else(|| OpenError::Zip(format!("entry {i} is missing")))?
                .map_err(|e| OpenError::Zip(e.to_string()))?
                .to_string();
            // OPC part names compare case-insensitively.
            index.insert(name.to_ascii_lowercase(), i);
            entries.push(name);
        }

        let mut package = Package {
            zip: zip.clone(),
            entries,
            index,
            content_types: ContentTypes::default(),
            rels: BTreeMap::new(),
            main_part: String::new(),
        };

        let ct_text = package.read_text_in(&mut archive, "[Content_Types].xml")?;
        package.content_types = ContentTypes::parse(&ct_text.text)?;

        let rels_parts: Vec<String> = package
            .entries
            .iter()
            .filter(|n| n.ends_with(".rels"))
            .cloned()
            .collect();
        for rels_part in rels_parts {
            let Some(source) = source_part_for(&rels_part) else {
                continue;
            };
            let text = package.read_text_in(&mut archive, &rels_part)?;
            let rels = parse_rels(&source, &rels_part, &text.text)?;
            package.rels.insert(source.to_ascii_lowercase(), rels);
        }
        if !package.rels.contains_key("") {
            return Err(OpenError::MissingPart("_rels/.rels".to_string()));
        }

        let package_rels = &package.rels[""];
        if package_rels
            .iter()
            .any(|r| r.rel_type == REL_OFFICE_DOCUMENT_STRICT)
        {
            return Err(OpenError::Unsupported {
                part: "_rels/.rels".to_string(),
                reason: "Strict Open XML conformance".to_string(),
            });
        }
        let main = package_rels
            .iter()
            .find(|r| r.rel_type == REL_OFFICE_DOCUMENT && !r.external)
            .and_then(|r| r.part.clone())
            .ok_or(OpenError::NoMainDocument)?;
        let main = package
            .actual_name(&main)
            .ok_or(OpenError::MissingMainPart(main.clone()))?
            .to_string();
        let ct = package.content_type(&main);
        if !ct.is_some_and(|ct| MAIN_CONTENT_TYPES.contains(&ct)) {
            return Err(OpenError::NotWordprocessing(ct.map(str::to_string)));
        }
        package.main_part = main;
        Ok(package)
    }

    fn read_text_in(
        &self,
        archive: &mut ZipArchive<Cursor<&[u8]>>,
        part: &str,
    ) -> Result<xml::XmlText, OpenError> {
        xml::decode(part, self.read_bytes_in(archive, part)?)
    }

    fn read_bytes_in(
        &self,
        archive: &mut ZipArchive<Cursor<&[u8]>>,
        part: &str,
    ) -> Result<Vec<u8>, OpenError> {
        let i = *self
            .index
            .get(&part.to_ascii_lowercase())
            .ok_or_else(|| OpenError::MissingPart(part.to_string()))?;
        let mut file = archive
            .by_index(i)
            .map_err(|e| OpenError::Zip(e.to_string()))?;
        // Not sized from the entry's header, which a crafted file can set to
        // anything.
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)
            .map_err(|e| OpenError::Zip(format!("{part}: {e}")))?;
        Ok(bytes)
    }

    pub fn read_text(&self, part: &str) -> Result<xml::XmlText, OpenError> {
        let mut archive = self.archive()?;
        self.read_text_in(&mut archive, part)
    }

    /// A part's bytes as the package stores them, decompressed.
    pub fn read_bytes(&self, part: &str) -> Result<Vec<u8>, OpenError> {
        let mut archive = self.archive()?;
        self.read_bytes_in(&mut archive, part)
    }

    pub(crate) fn archive(&self) -> Result<ZipArchive<Cursor<&[u8]>>, OpenError> {
        ZipArchive::new(Cursor::new(&self.zip[..])).map_err(|e| OpenError::Zip(e.to_string()))
    }

    pub fn main_part(&self) -> &str {
        &self.main_part
    }

    pub fn entries(&self) -> &[String] {
        &self.entries
    }

    /// The entry name as stored, for a part name in any case.
    pub fn actual_name(&self, part: &str) -> Option<&str> {
        self.index
            .get(&part.to_ascii_lowercase())
            .map(|&i| self.entries[i].as_str())
    }

    pub fn content_type(&self, part: &str) -> Option<&str> {
        self.content_types.get(part)
    }

    pub fn relationships(&self, part: &str) -> &[Relationship] {
        self.rels
            .get(&part.to_ascii_lowercase())
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub fn relationship(&self, part: &str, id: &str) -> Option<&Relationship> {
        self.relationships(part).iter().find(|r| r.id == id)
    }

    /// The first internal target of a relationship type from `part`, if it exists in the package.
    pub fn related_part(&self, part: &str, rel_type: &str) -> Option<String> {
        self.relationships(part)
            .iter()
            .filter(|r| r.rel_type == rel_type && !r.external)
            .find_map(|r| r.part.as_deref().and_then(|p| self.actual_name(p)))
            .map(str::to_string)
    }

    pub(crate) fn all_relationships(&self) -> impl Iterator<Item = (&str, &Relationship)> {
        self.rels
            .iter()
            .flat_map(|(source, rels)| rels.iter().map(move |r| (source.as_str(), r)))
    }

    pub fn len_bytes(&self) -> usize {
        self.zip.len()
    }
}
