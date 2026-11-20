//! Comment markers, thread metadata and their package parts.
//!
//! A comment is a `w:comment` in the comments part, and three markers with
//! its ID in the story it comments on: a range start, a range end, and a
//! run holding its reference. A reply is a comment whose thread the
//! `commentsExtended` part records: the `w14:paraId` of its last paragraph
//! names the last paragraph of the comment it answers. Word keeps a thread
//! flat, so every reply answers the first comment of its thread, and its
//! markers sit beside that comment's.
//!
//! A document without comments gets a comments part when it opens, which
//! export writes once it holds a comment. Comment IDs a command takes are
//! one above every ID a comment or comment marker has, and paragraph IDs
//! come from the comment's content alone for deterministic serialization.

use std::collections::BTreeMap;

use crate::edit::{check_text, Point, Track};

use crate::error::{EditError, ExportError, OpenError};
use crate::export::{prefix, Additions, DECLARATION};
use crate::import::{IN_COMMENT, NO_COMMAND};
use crate::ir::{Made, NodeId, NodeKind, ParagraphInfo, StoryKind, Support};
use crate::structure::{refuse, Loc};
use crate::xml::{self, escape_attr, XmlText, W, W14};
use crate::{fnv1a, Document, Theme};

pub(crate) const REL_COMMENTS_EXTENDED: &str =
    "http://schemas.microsoft.com/office/2011/relationships/commentsExtended";
pub(crate) const CT_COMMENTS: &str =
    "application/vnd.openxmlformats-officedocument.wordprocessingml.comments+xml";
pub(crate) const CT_COMMENTS_EXTENDED: &str =
    "application/vnd.openxmlformats-officedocument.wordprocessingml.commentsExtended+xml";
const W15: &str = "http://schemas.microsoft.com/office/word/2012/wordml";
const MC: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";

/// Which of a comment's markers a node is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mark {
    Start,
    End,
    Reference,
}

/// Where a new comment's markers go: its range start, and its range end
/// with the reference after it.
pub(crate) struct Places {
    pub start: Point,
    pub end: Point,
}

fn refused<T>(comment: impl ToString, reason: &'static str) -> Result<T, EditError> {
    Err(EditError::Comment {
        comment: comment.to_string(),
        reason,
    })
}

/// A paragraph ID for line `index` of comment `comment`, from its content
/// alone. Word's paragraph IDs are below `0x80000000` and not zero.
fn para_id(comment: u32, index: usize, by: &Track, line: &str) -> String {
    let key = format!("{comment}\0{index}\0{}\0{}\0{line}", by.author, by.date);
    let id = (fnv1a(key.as_bytes()) as u32) & 0x7FFF_FFFF;
    format!("{:08X}", id.max(1))
}

pub(crate) fn check_comment_text(text: &str, initials: Option<&str>) -> Result<(), EditError> {
    for line in text.split('\n') {
        check_text(line)?;
    }
    if let Some(initials) = initials {
        check_text(initials)?;
    }
    Ok(())
}

impl Document {
    /// Which comment marker a node is, and the comment's ID.
    pub(crate) fn comment_mark(&self, id: NodeId) -> Option<(Mark, &str)> {
        let NodeKind::Marker {
            element,
            marker_id: Some(comment),
            ..
        } = &self.n(id).kind
        else {
            return None;
        };
        let mark = match element.rsplit(':').next() {
            Some("commentRangeStart") => Mark::Start,
            Some("commentRangeEnd") => Mark::End,
            Some("commentReference") => Mark::Reference,
            _ => return None,
        };
        Some((mark, comment))
    }

    /// Gives a document without a comments part one to add comments to, and
    /// reads the threads `commentsExtended` records.
    pub(crate) fn open_comments(&mut self, main: &str, theme: &Theme) -> Result<(), OpenError> {
        if self.comments_story().is_none() {
            let dir = main.rfind('/').map_or("", |i| &main[..=i]);
            let part = (0..)
                .map(|n| match n {
                    0 => format!("{dir}comments.xml"),
                    n => format!("{dir}comments{n}.xml"),
                })
                .find(|p| self.package.actual_name(p).is_none())
                .expect("a free part name");
            let text = format!(
                "{DECLARATION}<w:comments xmlns:mc=\"{MC}\" xmlns:w=\"{W}\" xmlns:w14=\"{W14}\" mc:Ignorable=\"w14\"></w:comments>"
            );
            let text = XmlText {
                text: text.into(),
                bom: false,
            };
            self.import_story_text(&part, text, StoryKind::Comments, theme, true)?;
        }
        let Some(part) = self.package.related_part(main, REL_COMMENTS_EXTENDED) else {
            return Ok(());
        };
        let text = self.package.read_text(&part)?;
        let parsed = xml::parse(&part, &text.text)?;
        let parents: BTreeMap<&str, &str> = parsed
            .descendants()
            .filter(|n| xml::is(*n, W15, "commentEx"))
            .filter_map(|n| {
                Some((
                    n.attribute((W15, "paraId"))?,
                    n.attribute((W15, "paraIdParent"))?,
                ))
            })
            .collect();
        let by_para: BTreeMap<String, String> = self
            .comments()
            .filter_map(|c| {
                Some((
                    self.last_para_id(c)?.to_string(),
                    self.comment_id(c).to_string(),
                ))
            })
            .collect();
        let comments: Vec<NodeId> = self.comments().collect();
        for comment in comments {
            let parent = self
                .last_para_id(comment)
                .and_then(|p| parents.get(p))
                .and_then(|p| by_para.get(*p))
                .cloned();
            if let NodeKind::Comment { parent: slot, .. } = &mut self.nodes[comment.index()].kind {
                *slot = parent;
            }
        }
        self.parsed_parts.push(part.clone());
        self.comments_extended = Some(part);
        Ok(())
    }

    /// The comments story's index.
    pub(crate) fn comments_story(&self) -> Option<usize> {
        self.stories.iter().position(|s| {
            matches!(
                self.n(s.root).kind,
                NodeKind::Story {
                    story: StoryKind::Comments,
                    ..
                }
            )
        })
    }

    /// The comments in the comments story, in order.
    pub(crate) fn comments(&self) -> impl Iterator<Item = NodeId> + '_ {
        let root = self.comments_story().map(|s| self.stories[s].root);
        root.into_iter()
            .flat_map(|r| self.n(r).children.iter().copied())
            .filter(|&c| matches!(self.n(c).kind, NodeKind::Comment { .. }))
    }

    pub(crate) fn comment_id(&self, comment: NodeId) -> &str {
        match &self.n(comment).kind {
            NodeKind::Comment { comment_id, .. } => comment_id,
            _ => unreachable!("a comment"),
        }
    }

    pub(crate) fn comment_parent(&self, comment: NodeId) -> Option<&str> {
        match &self.n(comment).kind {
            NodeKind::Comment { parent, .. } => parent.as_deref(),
            _ => None,
        }
    }

    /// The `w14:paraId` of a comment's last paragraph, which a reply names.
    pub(crate) fn last_para_id(&self, comment: NodeId) -> Option<&str> {
        let last = self
            .n(comment)
            .children
            .iter()
            .rev()
            .find_map(|&c| match &self.n(c).kind {
                NodeKind::Paragraph(info) => Some(info),
                _ => None,
            })?;
        last.word_para_id.as_deref()
    }

    pub(crate) fn find_comment(&self, comment: &str) -> Option<NodeId> {
        self.comments().find(|&c| self.comment_id(c) == comment)
    }

    /// `wanted` if no comment or comment marker has it, else one above
    /// every ID they have.
    pub(crate) fn free_comment_id(&self, wanted: Option<u32>) -> Result<u32, EditError> {
        let mut taken = std::collections::BTreeSet::new();
        for node in &self.nodes {
            if node.made.is_some() && self.attached(node.id).is_err() {
                continue;
            }
            let id = match &node.kind {
                NodeKind::Comment { comment_id, .. } => comment_id.as_str(),
                _ => match self.comment_mark(node.id) {
                    Some((_, id)) => id,
                    None => continue,
                },
            };
            taken.extend(id.trim().parse::<u32>().ok());
        }
        if let Some(id) = wanted.filter(|id| !taken.contains(id)) {
            return Ok(id);
        }
        let next = taken.last().map_or(Some(0), |n| n.checked_add(1));
        next.ok_or(EditError::Comment {
            comment: u32::MAX.to_string(),
            reason: "has no comment ID left to take",
        })
    }

    /// The highest ID a comment a command made has, which export numbers
    /// revisions above.
    pub(crate) fn made_comment_ids(&self) -> u32 {
        self.comments()
            .filter(|&c| self.n(c).made.is_some())
            .filter_map(|c| self.comment_id(c).parse::<u32>().ok())
            .max()
            .unwrap_or(0)
    }

    /// Where a comment marker goes for a point: beside the runs it is at
    /// the edge of, in the paragraph, hyperlink or insertion holding them.
    /// Refuses a point no comment can be at.
    pub(crate) fn mark_place(&self, at: Point) -> Result<Loc, EditError> {
        let loc = self.edge(self.locate(at)?)?;
        let paragraph = self.paragraph_at(loc)?;
        let story = &self
            .n(self.stories[self.n(paragraph).story as usize].root)
            .kind;
        match story {
            NodeKind::Story {
                story: StoryKind::Header | StoryKind::Footer,
                ..
            } => {
                return refuse(
                    paragraph,
                    "Word does not allow comments in headers and footers",
                )
            }
            NodeKind::Story {
                story: StoryKind::Comments,
                ..
            } => return refuse(paragraph, "a comment cannot comment on a comment"),
            _ => {}
        }
        let loc = self.lift_boundary(loc, paragraph);
        let (Loc::Text(node, _) | Loc::Child(node, _)) = loc;
        self.check_editable(node)?;
        let (container, index) = match loc {
            Loc::Text(text, _) => (self.n(text).parent.expect("attached"), self.index_of(text)),
            Loc::Child(container, index) => (container, index),
        };
        self.check_outside_fields(container, index)?;
        Ok(loc)
    }

    /// Where a reply to `root` puts its markers: its start after the
    /// thread's starts, and its end and reference after the thread's.
    pub(crate) fn reply_places(&self, root: &str) -> Result<Places, EditError> {
        let thread: Vec<String> = self
            .comments()
            .filter(|&c| self.comment_id(c) == root || self.comment_parent(c) == Some(root))
            .map(|c| self.comment_id(c).to_string())
            .collect();
        let in_thread = |id: NodeId, want: Mark| match self.comment_mark(id) {
            Some((mark, c)) => mark == want && thread.iter().any(|t| t == c),
            None => false,
        };
        let marker = |want: Mark| {
            self.stories
                .iter()
                .filter(|s| {
                    !matches!(
                        self.n(s.root).kind,
                        NodeKind::Story {
                            story: StoryKind::Comments,
                            ..
                        }
                    )
                })
                .flat_map(|s| self.descendants(s.root))
                .find(|&n| self.comment_mark(n) == Some((want, root)))
        };
        let (Some(start), Some(reference)) = (marker(Mark::Start), marker(Mark::Reference)) else {
            return refused(root, "has no range in the document to reply beside");
        };
        // The last sibling from `from` on that `keep` holds.
        let last = |from: NodeId, keep: &dyn Fn(NodeId) -> bool| {
            let parent = self.n(from).parent.expect("attached");
            let siblings = &self.n(parent).children[self.index_of(from) + 1..];
            siblings
                .iter()
                .take_while(|&&s| keep(s))
                .last()
                .copied()
                .unwrap_or(from)
        };
        let start = last(start, &|s| in_thread(s, Mark::Start));
        let reference = match self.n(reference).parent {
            Some(run) if matches!(self.n(run).kind, NodeKind::Run(_)) => run,
            _ => reference,
        };
        let end = last(reference, &|s| {
            in_thread(s, Mark::End)
                || matches!(self.n(s).kind, NodeKind::Run(_))
                    && !self.n(s).children.is_empty()
                    && self
                        .n(s)
                        .children
                        .iter()
                        .all(|&c| in_thread(c, Mark::Reference))
        });
        Ok(Places {
            start: Point::After { after: start },
            end: Point::After { after: end },
        })
    }

    pub(crate) fn compose_comment(
        &mut self,
        comment: u32,
        text: &str,
        by: &Track,
        initials: Option<&str>,
        parent: Option<&str>,
    ) -> NodeId {
        let story = self
            .comments_story()
            .expect("a document has a comments story");
        let root = self.stories[story].root;
        let story = story as u32;
        let name = self.w_name(root, "comment");
        let (p, declare) = match name.split_once(':') {
            Some((p, _)) => (format!("{p}:"), String::new()),
            None => ("w:".to_string(), format!(" xmlns:w=\"{W}\"")),
        };
        let mut head = format!(
            "<{name}{declare} {p}id=\"{comment}\" {p}author=\"{}\" {p}date=\"{}\"",
            escape_attr(&by.author),
            escape_attr(&by.date)
        );
        if let Some(initials) = initials {
            head.push_str(&format!(" {p}initials=\"{}\"", escape_attr(initials)));
        }
        head.push('>');
        let made = Made {
            head,
            tail: format!("</{name}>"),
            like: None,
        };
        let kind = NodeKind::Comment {
            comment_id: comment.to_string(),
            author: Some(by.author.clone()),
            date: Some(by.date.clone()),
            initials: initials.map(str::to_string),
            parent: parent.map(str::to_string),
        };
        let node = self.create(kind, Support::ReadOnly(IN_COMMENT), made, story);
        let (paragraph, run, t) = (
            self.w_name(root, "p"),
            self.w_name(root, "r"),
            self.w_name(root, "t"),
        );
        for (index, line) in text.split('\n').enumerate() {
            let id = para_id(comment, index, by, line);
            let made = Made {
                head: format!("<{paragraph} xmlns:w14=\"{W14}\" w14:paraId=\"{id}\">"),
                tail: format!("</{paragraph}>"),
                like: None,
            };
            let info = ParagraphInfo {
                word_para_id: Some(id),
                ..ParagraphInfo::default()
            };
            let p = self.create(
                NodeKind::Paragraph(Box::new(info)),
                Support::ReadOnly(IN_COMMENT),
                made,
                story,
            );
            if !line.is_empty() {
                let made = Made {
                    head: format!("<{run}>"),
                    tail: format!("</{run}>"),
                    like: None,
                };
                let r = self.create(
                    NodeKind::Run(Box::default()),
                    Support::ReadOnly(IN_COMMENT),
                    made,
                    story,
                );
                let made = Made {
                    head: format!("<{t} xml:space=\"preserve\">"),
                    tail: format!("</{t}>"),
                    like: None,
                };
                let kind = NodeKind::Text {
                    text: line.to_string(),
                };
                let leaf = self.create(kind, Support::ReadOnly(IN_COMMENT), made, story);
                self.attach(leaf, r, 0);
                self.attach(r, p, 0);
            }
            let at = self.n(node).children.len();
            self.attach(p, node, at);
        }
        let index = self
            .n(root)
            .children
            .iter()
            .position(|&c| {
                matches!(self.n(c).kind, NodeKind::Comment { .. })
                    && self.comment_id(c).parse::<u32>().is_ok_and(|c| c > comment)
            })
            .unwrap_or(self.n(root).children.len());
        self.attach(node, root, index);
        node
    }

    pub(crate) fn loose_comment_marks(
        &mut self,
        container: NodeId,
        comment: u32,
        end: bool,
    ) -> Vec<NodeId> {
        let story = self.n(container).story;
        let id = self.w_attr_name(container, "id");
        let marker = |doc: &mut Self, local: &str| {
            let element = doc.w_name(container, local);
            let made = Made {
                head: format!("<{element} {id}=\"{comment}\"/>"),
                tail: String::new(),
                like: None,
            };
            let kind = NodeKind::Marker {
                element,
                marker_id: Some(comment.to_string()),
                name: None,
            };
            doc.create(kind, Support::ReadOnly(NO_COMMAND), made, story)
        };
        let first = marker(
            self,
            if end {
                "commentRangeEnd"
            } else {
                "commentRangeStart"
            },
        );
        let mut nodes = vec![first];
        if end {
            let reference = marker(self, "commentReference");
            let name = self.w_name(container, "r");
            let made = Made {
                head: format!("<{name}>"),
                tail: format!("</{name}>"),
                like: None,
            };
            let run = self.create(
                NodeKind::Run(Box::default()),
                Support::Editable,
                made,
                story,
            );
            self.attach(reference, run, 0);
            nodes.push(run);
        }
        nodes
    }
}

/// What export adds for comments.
impl Document {
    /// The comments part the engine made, while it holds a comment.
    pub(crate) fn added_comments(&self) -> Option<&crate::Story> {
        let story = &self.stories[self.comments_story()?];
        (story.added && !self.n(story.root).children.is_empty()).then_some(story)
    }

    /// The comments commands made that export records threads for, in ID
    /// order.
    fn made_comments(&self) -> Vec<NodeId> {
        let mut made: Vec<NodeId> = self
            .comments()
            .filter(|&c| self.n(c).made.is_some())
            .collect();
        made.sort_by_key(|&c| self.comment_id(c).parse::<u32>().unwrap_or(0));
        made
    }

    /// The `commentsExtended` part export writes, and whether it adds it.
    pub(crate) fn threads_part(&self) -> Option<(String, bool)> {
        if self.made_comments().is_empty() {
            return None;
        }
        if let Some(part) = &self.comments_extended {
            return Some((part.clone(), false));
        }
        let main = self.package.main_part();
        let dir = main.rfind('/').map_or("", |i| &main[..=i]);
        let part = (0..)
            .map(|n| match n {
                0 => format!("{dir}commentsExtended.xml"),
                n => format!("{dir}commentsExtended{n}.xml"),
            })
            .find(|p| self.package.actual_name(p).is_none())
            .expect("a free part name");
        Some((part, true))
    }

    pub(crate) fn write_threads(
        &self,
        part: String,
        added: bool,
        out: &mut Additions,
    ) -> Result<(), ExportError> {
        let entries = |p: &str, declare: &str| -> String {
            self.made_comments()
                .into_iter()
                .filter_map(|c| {
                    let id = self.last_para_id(c)?;
                    let parent = self
                        .comment_parent(c)
                        .and_then(|r| self.find_comment(r))
                        .and_then(|r| self.last_para_id(r))
                        .map_or(String::new(), |r| format!(" {p}paraIdParent=\"{r}\""));
                    Some(format!(
                        "<{p}commentEx{declare} {p}paraId=\"{id}\"{parent} {p}done=\"0\"/>"
                    ))
                })
                .collect()
        };
        if added {
            let text = format!(
                "{DECLARATION}<w15:commentsEx xmlns:mc=\"{MC}\" xmlns:w15=\"{W15}\" mc:Ignorable=\"w15\">{}</w15:commentsEx>",
                entries("w15:", "")
            );
            out.added.push((part, text));
        } else {
            let text = self.edit_root(&part, |root, _, close| {
                let markup = match prefix(root, W15).as_str() {
                    "" => entries("w15:", &format!(" xmlns:w15=\"{W15}\"")),
                    p => entries(p, ""),
                };
                vec![(close, markup)]
            })?;
            out.replaced.insert(part.to_ascii_lowercase(), text);
        }
        Ok(())
    }
}
