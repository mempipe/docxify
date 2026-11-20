//! Package validation: OPC structure and reference integrity.
//!
//! This checks what the engine itself can break: the ZIP container, content
//! types, relationship targets, XML well-formedness, and that every
//! relationship ID a part uses exists. It is not WordprocessingML schema
//! validation.

use std::collections::{HashMap, HashSet};
use std::io::{Cursor, Read};

use serde::Serialize;
use zip::ZipArchive;

use crate::diagnostics::Severity;
use crate::package::{
    parse_rels, source_part_for, ContentTypes, Relationship, REL_OFFICE_DOCUMENT,
};
use crate::xml;

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Serialize)]
pub struct Issue {
    pub severity: Severity,
    pub code: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub part: Option<String>,
    pub message: String,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Serialize)]
pub struct ValidationReport {
    pub valid: bool,
    pub parts: usize,
    pub issues: Vec<Issue>,
}

pub fn validate_package(bytes: &[u8]) -> ValidationReport {
    let mut issues = Vec::new();
    let mut error = |code, part: Option<&str>, message: String| {
        issues.push(Issue {
            severity: Severity::Error,
            code,
            part: part.map(str::to_string),
            message,
        })
    };

    let mut archive = match ZipArchive::new(Cursor::new(bytes)) {
        Ok(a) => a,
        Err(e) => {
            error("zip", None, e.to_string());
            return ValidationReport {
                valid: false,
                parts: 0,
                issues,
            };
        }
    };

    let mut parts: Vec<String> = Vec::new();
    let mut contents: HashMap<String, Vec<u8>> = HashMap::new();
    let mut seen = HashSet::new();
    for i in 0..archive.len() {
        let mut file = match archive.by_index(i) {
            Ok(f) => f,
            Err(e) => {
                error("zip", None, format!("entry {i}: {e}"));
                continue;
            }
        };
        let name = match file.name() {
            Ok(n) => n.to_string(),
            Err(e) => {
                error("zip", None, format!("entry {i}: {e}"));
                continue;
            }
        };
        if name.ends_with('/') {
            continue;
        }
        if !seen.insert(name.to_ascii_lowercase()) {
            error(
                "duplicate_part",
                Some(&name),
                "two entries share this part name".to_string(),
            );
        }
        let mut data = Vec::new();
        if let Err(e) = file.read_to_end(&mut data) {
            error("zip", Some(&name), e.to_string());
        }
        contents.insert(name.to_ascii_lowercase(), data);
        parts.push(name);
    }
    let exists = |part: &str| contents.contains_key(&part.to_ascii_lowercase());

    let content_types = match contents.get("[content_types].xml") {
        None => {
            error(
                "missing_part",
                None,
                "[Content_Types].xml is missing".to_string(),
            );
            ContentTypes::default()
        }
        Some(data) => match xml::decode("[Content_Types].xml", data.clone())
            .and_then(|t| ContentTypes::parse(&t.text))
        {
            Ok(ct) => ct,
            Err(e) => {
                error("malformed_xml", Some("[Content_Types].xml"), e.to_string());
                ContentTypes::default()
            }
        },
    };

    let mut rels: HashMap<String, Vec<Relationship>> = HashMap::new();
    for part in &parts {
        if part != "[Content_Types].xml" && content_types.get(part).is_none() {
            error(
                "missing_content_type",
                Some(part),
                "no Default or Override covers this part".to_string(),
            );
        }
        let data = &contents[&part.to_ascii_lowercase()];
        let looks_xml = part.ends_with(".xml")
            || part.ends_with(".rels")
            || content_types
                .get(part)
                .is_some_and(|ct| ct.ends_with("xml"));
        if !looks_xml {
            continue;
        }
        let text = match xml::decode(part, data.clone()) {
            Ok(t) => t,
            Err(e) => {
                error("malformed_xml", Some(part), e.to_string());
                continue;
            }
        };
        if let Err(e) = xml::parse(part, &text.text) {
            error("malformed_xml", Some(part), e.to_string());
            continue;
        }
        if let Some(source) = source_part_for(part) {
            if !source.is_empty() && !exists(&source) {
                error(
                    "orphan_relationships",
                    Some(part),
                    format!("describes {source}, which is missing"),
                );
            }
            match parse_rels(&source, part, &text.text) {
                Ok(list) => {
                    for r in list.iter().filter(|r| !r.external) {
                        let target = r.part.as_deref().unwrap_or_default();
                        if !exists(target) {
                            error(
                                "broken_relationship",
                                Some(part),
                                format!(
                                    "{} ({}) targets {target}, which is missing",
                                    r.id, r.rel_type
                                ),
                            );
                        }
                    }
                    rels.insert(source.to_ascii_lowercase(), list);
                }
                Err(e) => error("malformed_xml", Some(part), e.to_string()),
            }
        }
    }

    match rels.get("") {
        None => error("missing_part", None, "_rels/.rels is missing".to_string()),
        Some(list) => match list.iter().find(|r| r.rel_type == REL_OFFICE_DOCUMENT) {
            None => error(
                "no_main_document",
                Some("_rels/.rels"),
                "no officeDocument relationship".to_string(),
            ),
            Some(main) => {
                let target = main.part.clone().unwrap_or_default();
                let ct = content_types.get(&target).unwrap_or_default();
                if exists(&target) && !ct.contains("wordprocessingml") && !ct.contains("ms-word") {
                    error(
                        "not_wordprocessing",
                        Some(&target),
                        format!("content type {ct:?}"),
                    );
                }
            }
        },
    }

    // Relationship IDs used inside XML parts must exist in that part's rels.
    for part in &parts {
        if part.ends_with(".rels") || !part.ends_with(".xml") {
            continue;
        }
        let Ok(text) = xml::decode(part, contents[&part.to_ascii_lowercase()].clone()) else {
            continue;
        };
        let Ok(doc) = xml::parse(part, &text.text) else {
            continue;
        };
        let ids: HashSet<&str> = rels
            .get(&part.to_ascii_lowercase())
            .map(|l| l.iter().map(|r| r.id.as_str()).collect())
            .unwrap_or_default();
        xml::check_relationship_ids(
            &doc,
            |id| ids.contains(id),
            |message| error("unresolved_relationship_id", Some(part), message),
        );
    }

    ValidationReport {
        valid: issues.iter().all(|i| i.severity != Severity::Error),
        parts: parts.len(),
        issues,
    }
}
