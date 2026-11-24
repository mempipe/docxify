//! The numbers Word knows drawings, comments, revisions and lists by: which
//! ones the opened file already uses, and which free one a new object takes.

use std::collections::BTreeSet;
use std::ops::RangeInclusive;

use crate::xml::{self, W, WP};

/// Word fails to load a file with a drawing ID of 0 or above 2147483646.
pub(crate) const DRAWING_IDS: RangeInclusive<u32> = 1..=2_147_483_646;

/// Word reads comment, revision and list numbers as 32-bit signed integers.
pub(crate) const HIGHEST_NUMBER: u32 = i32::MAX as u32;

/// The numbers the opened file uses, in markup the engine models and in
/// markup it only preserves. A new object that took one would join whatever
/// already names it.
#[derive(Clone, Default)]
pub(crate) struct SourceIds {
    /// `wp:docPr` IDs.
    pub drawings: BTreeSet<u32>,
    /// The IDs of comments and comment markers.
    pub comments: BTreeSet<u32>,
    /// Every `w:id`: comments, revisions, bookmarks and other ranges.
    pub marks: BTreeSet<u32>,
    /// List instance IDs, both assigned and referenced.
    pub lists: BTreeSet<u32>,
    /// Abstract numbering IDs, both assigned and referenced.
    pub abstracts: BTreeSet<u32>,
}

impl SourceIds {
    pub fn read(&mut self, part: &roxmltree::Document) {
        let number = |value: Option<&str>| value?.trim().parse::<u32>().ok();
        for el in part.descendants().filter(|n| n.is_element()) {
            if xml::is(el, WP, "docPr") {
                self.drawings.extend(number(el.attribute("id")));
            } else if el.tag_name().namespace() == Some(W) {
                match el.tag_name().name() {
                    "num" => self.lists.extend(number(el.attribute((W, "numId")))),
                    "numId" => self.lists.extend(number(el.attribute((W, "val")))),
                    "abstractNum" => self
                        .abstracts
                        .extend(number(el.attribute((W, "abstractNumId")))),
                    "abstractNumId" => self.abstracts.extend(number(el.attribute((W, "val")))),
                    _ => {}
                }
            }
            let Some(id) = number(el.attribute((W, "id"))) else {
                continue;
            };
            self.marks.insert(id);
            let comment = [
                "comment",
                "commentRangeStart",
                "commentRangeEnd",
                "commentReference",
            ];
            if comment.iter().any(|name| xml::is(el, W, name)) {
                self.comments.insert(id);
            }
        }
    }
}

/// One above the highest number in use, or the lowest free number when Word
/// accepts nothing higher.
pub(crate) fn next_free(used: &BTreeSet<u32>, accepted: RangeInclusive<u32>) -> Option<u32> {
    let above = match used.last() {
        Some(highest) => highest.checked_add(1),
        None => Some(0),
    };
    above
        .map(|n| n.max(*accepted.start()))
        .filter(|n| accepted.contains(n))
        .or_else(|| accepted.clone().find(|n| !used.contains(n)))
}
