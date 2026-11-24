//! Run and paragraph properties are edited as markup so that unrelated
//! properties retain their source bytes. The XML parser that import uses
//! says where each property is and what namespace names it.

use std::ops::Range;

use crate::edit::{Format, Point};
use crate::error::EditError;
use crate::ir::{NodeId, NodeKind, ParaProps, RunInfo, RunProps};
use crate::revisions::REVISION_ID;
use crate::structure::refuse;
use crate::xml::{self, escape_attr, w_local, W};
use crate::Document;

/// What the revision-ID placeholder is read as. XML cannot hold the
/// placeholder, and this is as long, so what the parser finds is where it is
/// in the markup that was given.
const READ_REVISION_ID: &str = "000";
const _: () = assert!(REVISION_ID.len_utf8() == READ_REVISION_ID.len());

/// Properties the XML parser cannot read where they are.
#[derive(Debug)]
pub(crate) struct Unreadable;

impl Unreadable {
    pub(crate) fn on(self, node: NodeId) -> EditError {
        EditError::Refused {
            node,
            reason: "the properties are not well-formed markup",
        }
    }
}

/// Namespace bindings for splicing OOXML property fragments. Values retain
/// their XML quotes so a fragment can be parsed in the same scope.
#[derive(Debug, Clone, Default)]
pub(crate) struct Scope(std::sync::Arc<std::collections::BTreeMap<String, String>>);

impl Scope {
    pub(crate) fn from_element(element: roxmltree::Node) -> Self {
        Self(std::sync::Arc::new(
            element
                .namespaces()
                .map(|ns| {
                    (
                        ns.name().unwrap_or("").to_owned(),
                        format!("\"{}\"", escape_attr(ns.uri())),
                    )
                })
                .collect(),
        ))
    }

    /// Adds what a start tag declares. A later declaration of a prefix
    /// hides an earlier one.
    fn declare(&mut self, tag: &str) {
        if tag.contains("xmlns") {
            for (name, value) in declarations(tag) {
                let prefix = name.strip_prefix("xmlns").unwrap_or_default();
                let prefix = prefix.strip_prefix(':').unwrap_or(prefix);
                std::sync::Arc::make_mut(&mut self.0).insert(prefix.to_owned(), value.to_owned());
            }
        }
    }

    /// This scope inside the element that `tag` starts.
    pub(crate) fn within(&self, tag: &str) -> Scope {
        let mut inner = self.clone();
        inner.declare(tag);
        inner
    }

    fn namespace(&self, prefix: &str) -> Option<&str> {
        self.0.get(prefix).map(|v| &v[1..v.len() - 1])
    }

    /// A qualified attribute, with a local declaration if no usable prefix is in scope.
    pub(crate) fn attribute(&self, ns: &str, preferred: &str, local: &str) -> String {
        let (declaration, prefix) = self.attribute_prefix(ns, preferred);
        format!("{declaration}{prefix}:{local}")
    }

    /// The declaration (including its trailing space) and prefix for attributes on a new element.
    pub(crate) fn attribute_prefix(&self, ns: &str, preferred: &str) -> (String, String) {
        if let Some(prefix) = self
            .0
            .keys()
            .find(|p| !p.is_empty() && self.namespace(p) == Some(ns))
        {
            return (String::new(), prefix.clone());
        }
        let mut prefix = preferred.to_owned();
        while self.0.contains_key(&prefix) {
            prefix.push('_');
        }
        (format!("xmlns:{prefix}=\"{ns}\" "), prefix)
    }

    /// `element`, written where this scope applies, with a declaration for
    /// each binding that `to` lacks or binds otherwise. It then reads the
    /// same where `to` applies.
    pub(crate) fn carried_to(&self, to: &Scope, element: &str) -> String {
        let Some(open) = tag_end(element, 0) else {
            return element.to_owned();
        };
        let own = Scope::default().within(&element[..open]);
        let mut missing: String = (self.0.iter())
            .filter(|(prefix, _)| !own.0.contains_key(*prefix))
            .filter(|(prefix, _)| to.namespace(prefix) != self.namespace(prefix))
            .map(|(prefix, value)| match prefix.is_empty() {
                true => format!(" xmlns={value}"),
                false => format!(" xmlns:{prefix}={value}"),
            })
            .collect();
        if !own.0.contains_key("")
            && !self.0.contains_key("")
            && to.namespace("").is_some_and(|ns| !ns.is_empty())
        {
            missing.push_str(" xmlns=\"\"");
        }
        let end = match element[..open].ends_with("/>") {
            true => open - 2,
            false => open - 1,
        };
        format!("{}{missing}{}", &element[..end], &element[end..])
    }

    /// Reads `markup`, one element where this scope applies, with the XML
    /// parser that import uses. `rest` completes markup that stops before
    /// its end tag, as a node's head does. `read` gets the element, and what
    /// to take from a range the parser gives to index `markup`. The element
    /// is read with the bindings its names can use, not all that are in
    /// scope: a document's root declares dozens.
    pub(crate) fn read<T>(
        &self,
        markup: &str,
        rest: &str,
        read: impl FnOnce(roxmltree::Node, usize) -> T,
    ) -> Result<T, Unreadable> {
        // A name can only use a prefix that the text spells before a colon.
        let spelled = |prefix: &str| {
            [markup, rest].iter().any(|text| {
                text.match_indices(prefix)
                    .any(|(at, _)| text.as_bytes().get(at + prefix.len()) == Some(&b':'))
            })
        };
        let mut wrapped = String::with_capacity(markup.len() + rest.len() + 256);
        wrapped.push_str("<scope");
        for (prefix, value) in self.0.iter() {
            if prefix.is_empty() {
                wrapped.push_str(" xmlns=");
                wrapped.push_str(value);
            } else if spelled(prefix) {
                wrapped.push_str(" xmlns:");
                wrapped.push_str(prefix);
                wrapped.push('=');
                wrapped.push_str(value);
            }
        }
        wrapped.push('>');
        let offset = wrapped.len();
        wrapped.push_str(markup);
        wrapped.push_str(rest);
        wrapped.push_str("</scope>");
        // Copied revisions acquire their actual IDs at export, and no reader
        // here consumes one.
        if wrapped.contains(REVISION_ID) {
            wrapped = wrapped.replace(REVISION_ID, READ_REVISION_ID);
        }
        let doc = xml::parse("properties", &wrapped).map_err(|_| Unreadable)?;
        let mut roots = xml::elements(doc.root_element());
        let root = roots.next().ok_or(Unreadable)?;
        if roots.next().is_some() {
            return Err(Unreadable);
        }
        Ok(read(root, offset))
    }
}

/// The namespace declarations of a start tag: each attribute name, and its
/// value as written, quotes included.
pub(crate) fn declarations(tag: &str) -> Vec<(&str, &str)> {
    let mut out = Vec::new();
    let mut at = 1 + qname(tag).len();
    while let Some(eq) = tag[at..].find('=').map(|eq| at + eq) {
        let name = tag[at..eq].trim();
        let start = eq + 1 + (tag[eq + 1..].len() - tag[eq + 1..].trim_start().len());
        let Some(quote) = tag[start..].chars().next() else {
            break;
        };
        let Some(close) = tag[start + 1..].find(quote).map(|c| start + 1 + c) else {
            break;
        };
        if name == "xmlns" || name.starts_with("xmlns:") {
            out.push((name, &tag[start..=close]));
        }
        at = close + 1;
    }
    out
}

/// The `w:rPr` children in the order the schema lists them.
const ORDER: [&str; 39] = [
    "rStyle",
    "rFonts",
    "b",
    "bCs",
    "i",
    "iCs",
    "caps",
    "smallCaps",
    "strike",
    "dstrike",
    "outline",
    "shadow",
    "emboss",
    "imprint",
    "noProof",
    "snapToGrid",
    "vanish",
    "webHidden",
    "color",
    "spacing",
    "w",
    "kern",
    "position",
    "sz",
    "szCs",
    "highlight",
    "u",
    "effect",
    "bdr",
    "shd",
    "fitText",
    "vertAlign",
    "rtl",
    "cs",
    "em",
    "lang",
    "eastAsianLayout",
    "specVanish",
    "oMath",
];

/// The `w:pPr` children in the order the schema lists them.
pub(crate) const PPR_ORDER: [&str; 36] = [
    "pStyle",
    "keepNext",
    "keepLines",
    "pageBreakBefore",
    "framePr",
    "widowControl",
    "numPr",
    "suppressLineNumbers",
    "pBdr",
    "shd",
    "tabs",
    "suppressAutoHyphens",
    "kinsoku",
    "wordWrap",
    "overflowPunct",
    "topLinePunct",
    "autoSpaceDE",
    "autoSpaceDN",
    "bidi",
    "adjustRightInd",
    "snapToGrid",
    "spacing",
    "ind",
    "contextualSpacing",
    "mirrorIndents",
    "suppressOverlap",
    "jc",
    "textDirection",
    "textAlignment",
    "textboxTightWrap",
    "outlineLvl",
    "divId",
    "cnfStyle",
    "rPr",
    "sectPr",
    "pPrChange",
];

const STYLE: [&str; 1] = ["pStyle"];
/// The `w:pPr` children a paragraph step may change.
const PARAGRAPH: [&str; 2] = ["pStyle", "numPr"];

/// The elements each property is written with. Word sets the complex-script
/// twin of bold and italic with it.
const BOLD: [&str; 2] = ["b", "bCs"];
const ITALIC: [&str; 2] = ["i", "iCs"];
const UNDERLINE: [&str; 1] = ["u"];
const RUN_STYLE: [&str; 1] = ["rStyle"];

impl Document {
    /// The story container's bindings, including the body's declarations,
    /// followed by nearer declarations on the path to this node.
    pub(crate) fn scope(&self, id: NodeId) -> Scope {
        let root = self.stories[self.n(id).story as usize].root;
        let mut chain = vec![id];
        while let Some(parent) = self.n(chain[chain.len() - 1]).parent {
            chain.push(parent);
        }
        if chain[chain.len() - 1] != root {
            chain.push(root);
        }
        let mut scope = self.stories[self.n(id).story as usize].scope.clone();
        for &id in chain.iter().rev().filter(|&&id| id != root) {
            let node = self.n(id);
            let head = self.tags(self.story_text(node), node).0;
            scope.declare(&head[..tag_end(&head, 0).unwrap_or(head.len())]);
            if id != chain[0] {
                self.declare_content_scope(id, &mut scope);
            }
        }
        scope
    }

    /// The namespace context of children. A control's XML content wrapper
    /// is preserved in its head and tail but has no node in the editing tree.
    pub(crate) fn content_scope(&self, id: NodeId) -> Scope {
        let mut scope = self.scope(id);
        self.declare_content_scope(id, &mut scope);
        scope
    }

    fn declare_content_scope(&self, id: NodeId, scope: &mut Scope) {
        let node = self.n(id);
        if let NodeKind::ContentControl {
            content_start: Some(start),
            ..
        } = node.kind
        {
            let text = self.story_text(node);
            let end = tag_end(text, start).expect("parsed control content tag");
            scope.declare(&text[start..end]);
        }
    }

    /// Read a property fragment with the same parser and namespace bindings as
    /// an imported element. Empty markup means the property was removed.
    pub(crate) fn read_markup<T>(
        &self,
        id: NodeId,
        markup: &str,
        read: impl FnOnce(roxmltree::Node) -> T,
    ) -> Result<Option<T>, EditError> {
        if markup.is_empty() {
            return Ok(None);
        }
        let scope = self.scope(id);
        let read = scope.read(markup, "", |root, _| read(root));
        read.map(Some).map_err(|e| e.on(id))
    }

    pub(crate) fn run_properties_from(
        &self,
        id: NodeId,
        properties: &str,
    ) -> Result<(Option<String>, RunProps), EditError> {
        Ok(self
            .read_markup(id, properties, |p| {
                (
                    crate::xml::w_val(p, "rStyle").map(str::to_owned),
                    crate::import::run_props(p, &self.theme),
                )
            })?
            .unwrap_or_default())
    }

    pub(crate) fn paragraph_properties_from(
        &self,
        id: NodeId,
        properties: &str,
    ) -> Result<(Option<String>, ParaProps), EditError> {
        Ok(self
            .read_markup(id, properties, |p| {
                (
                    crate::xml::w_val(p, "pStyle").map(str::to_owned),
                    crate::import::para_props(p),
                )
            })?
            .unwrap_or_default())
    }

    pub(crate) fn styled(&self, paragraph: NodeId, style: &str) -> Result<String, EditError> {
        let default = self.styles.default_paragraph_style().map(|s| s.id.as_str());
        let name = self.w_name(paragraph, "pPr");
        let set = (Some(style) != default).then(|| {
            let element = self.w_name(paragraph, "pStyle");
            let val = self.w_attr_name(paragraph, "val");
            vec![format!("<{element} {val}=\"{}\"/>", escape_attr(style))]
        });
        rewrite(
            &self.properties(paragraph, "pPr")?,
            &self.scope(paragraph),
            &name,
            &PPR_ORDER,
            &STYLE,
            set.unwrap_or_default(),
        )
        .map_err(|e| e.on(paragraph))
    }

    /// The paragraphs from the one holding `from` to the one holding `to`,
    /// in document order, those in tables between them included, when the
    /// two are in one story and in that order.
    pub(crate) fn paragraph_range(&self, from: Point, to: Point) -> Result<Vec<NodeId>, EditError> {
        let (a, b) = (self.edge(self.locate(from)?)?, self.edge(self.locate(to)?)?);
        let (pa, pb) = (self.paragraph_at(a)?, self.paragraph_at(b)?);
        if self.n(pa).story != self.n(pb).story {
            return refuse(pb, "the range ends in another story");
        }
        if self.path(b) < self.path(a) {
            return refuse(pa, "the range ends before it starts");
        }
        Ok(self.paragraphs_between(pa, pb))
    }

    /// The paragraphs from `first` to `last` in document order, those in
    /// tables between them included.
    pub(crate) fn paragraphs_between(&self, first: NodeId, last: NodeId) -> Vec<NodeId> {
        let mut stack = Vec::new();
        let mut node = first;
        while let Some(parent) = self.n(node).parent {
            stack.push((parent, self.index_of(node)));
            node = parent;
        }
        stack.reverse();
        let mut out = Vec::new();
        while let Some(&(container, i)) = stack.last() {
            let Some(&child) = self.n(container).children.get(i) else {
                stack.pop();
                if let Some(top) = stack.last_mut() {
                    top.1 += 1;
                }
                continue;
            };
            let paragraph = matches!(self.n(child).kind, NodeKind::Paragraph(_));
            if paragraph {
                out.push(child);
                if child == last {
                    break;
                }
            }
            if !paragraph && !self.n(child).children.is_empty() {
                stack.push((child, 0));
            } else if let Some(top) = stack.last_mut() {
                top.1 += 1;
            }
        }
        out
    }

    /// The run's `w:rPr` with `change` applied. A property that its style
    /// already gives is removed rather than repeated, as Word does.
    pub(crate) fn formatted(&self, run: NodeId, change: Format) -> Result<String, EditError> {
        let NodeKind::Run(info) = &self.n(run).kind else {
            unreachable!("a run");
        };
        let paragraph = self.paragraph_of(run);
        let styled = self.resolve_run(
            paragraph,
            Some(&RunInfo {
                style: info.style.clone(),
                ..RunInfo::default()
            }),
        );
        let name = self.w_name(run, "rPr");
        let prefix = name.strip_suffix("rPr").unwrap_or_default();
        let val = self.w_attr_name(run, "val");
        let toggle = |on: bool, names: &[&str]| -> Vec<String> {
            names
                .iter()
                .map(|n| match on {
                    true => format!("<{prefix}{n}/>"),
                    false => format!("<{prefix}{n} {val}=\"0\"/>"),
                })
                .collect()
        };
        let scope = &self.scope(run);
        let mut rpr = self.properties(run, "rPr")?;
        let mut set = |names: &[&str], set: Option<Vec<String>>| {
            let set = set.unwrap_or_default();
            rpr = rewrite(&rpr, scope, &name, &ORDER, names, set).map_err(|e| e.on(run))?;
            Ok::<_, EditError>(())
        };
        if let Some(on) = change.bold {
            set(
                &BOLD,
                (on != styled.bold.unwrap_or(false)).then(|| toggle(on, &BOLD)),
            )?;
        }
        if let Some(on) = change.italic {
            set(
                &ITALIC,
                (on != styled.italic.unwrap_or(false)).then(|| toggle(on, &ITALIC)),
            )?;
        }
        if let Some(on) = change.underline {
            let value = if on { "single" } else { "none" };
            set(
                &UNDERLINE,
                (on != underlined(&styled))
                    .then(|| vec![format!("<{prefix}u {val}=\"{value}\"/>")]),
            )?;
        }
        Ok(rpr)
    }

    /// The run's `w:rPr` with the character style `style`, or with none.
    pub(crate) fn run_styled(&self, run: NodeId, style: Option<&str>) -> Result<String, EditError> {
        let name = self.w_name(run, "rPr");
        let set = style.map(|style| {
            let element = self.w_name(run, "rStyle");
            let val = self.w_attr_name(run, "val");
            vec![format!("<{element} {val}=\"{}\"/>", escape_attr(style))]
        });
        let current = self.properties(run, "rPr")?;
        rewrite(
            &current,
            &self.scope(run),
            &name,
            &ORDER,
            &RUN_STYLE,
            set.unwrap_or_default(),
        )
        .map_err(|e| e.on(run))
    }

    /// A node's properties element, `w:rPr` or `w:pPr`, as markup, or
    /// nothing.
    pub(crate) fn properties(&self, id: NodeId, local: &str) -> Result<String, EditError> {
        let node = self.n(id);
        let (head, tail) = self.tags(self.story_text(node), node);
        let (_, found) =
            find_properties(&head, &tail, &self.scope(id), local).map_err(|e| e.on(id))?;
        Ok(found.map_or(String::new(), |range| head[range].to_string()))
    }

    /// A run's or hyperlink's start tag and run properties, without the
    /// other markup before its first child.
    pub(crate) fn bare_head(&self, id: NodeId) -> Result<String, EditError> {
        let node = self.n(id);
        let (head, tail) = self.tags(self.story_text(node), node);
        let found = find_properties(&head, &tail, &self.scope(id), "rPr").map_err(|e| e.on(id))?;
        Ok(match found {
            (open, Some(properties)) => format!("{}{}", &head[..open], &head[properties]),
            (open, None) => head[..open].to_string(),
        })
    }

    /// The markup between a container's last child and its end tag.
    pub(crate) fn trail(&self, id: NodeId) -> String {
        let node = self.n(id);
        let tail = self.tags(self.story_text(node), node).1;
        tail[..tail.rfind("</").unwrap_or(0)].to_string()
    }

    /// Whether a run or hyperlink holds an XML comment or a processing
    /// instruction before its first child, or more than whitespace after
    /// its last. Parts of its content must not repeat or drop that markup.
    pub(crate) fn has_edge_markup(&self, id: NodeId) -> bool {
        let node = self.n(id);
        let (head, tail) = self.tags(self.story_text(node), node);
        head.contains("<!--")
            || head.contains("<?")
            || !tail[..tail.rfind("</").unwrap_or(0)].trim().is_empty()
    }

    /// A source node's properties element as its source has it, whatever
    /// commands changed since.
    pub(crate) fn source_properties(&self, id: NodeId, local: &str) -> Result<String, EditError> {
        let node = self.n(id);
        let (head, tail) = self.source_tags(self.story_text(node), node);
        let (_, found) =
            find_properties(&head, &tail, &self.scope(id), local).map_err(|e| e.on(id))?;
        Ok(found.map_or(String::new(), |range| head[range].to_string()))
    }

    /// Writes final run properties while a builder is assembling a tree.
    /// Unlike a user step, this also takes a detached run.
    pub(crate) fn compose_run_properties(
        &mut self,
        id: NodeId,
        properties: &str,
    ) -> Result<(), EditError> {
        let changeable: Vec<&str> = BOLD
            .iter()
            .chain(&ITALIC)
            .chain(&UNDERLINE)
            .chain(&RUN_STYLE)
            .copied()
            .collect();
        let parsed = self.run_properties_from(id, properties)?;
        self.replace_properties(
            id,
            "rPr",
            properties,
            |e| w_local(e).is_some_and(|n| changeable.contains(&n)),
            "only bold, italic, underline and the character style can change this way",
        )?;
        if let NodeKind::Run(info) = &mut self.nodes[id.index()].kind {
            (info.style, info.props) = parsed;
        }
        Ok(())
    }

    /// Writes final paragraph properties before or after arrangement.
    pub(crate) fn compose_paragraph_properties(
        &mut self,
        id: NodeId,
        properties: &str,
    ) -> Result<(), EditError> {
        let parsed = self.paragraph_properties_from(id, properties)?;
        self.replace_properties(
            id,
            "pPr",
            properties,
            |e| w_local(e).is_some_and(|n| PARAGRAPH.contains(&n)),
            "only the paragraph style and numbering can change this way",
        )?;
        if let NodeKind::Paragraph(info) = &mut self.nodes[id.index()].kind {
            (info.style, info.props) = parsed;
        }
        Ok(())
    }

    /// Replaces a properties element if the remaining children match after
    /// filtering the permitted changes, each read by its namespace.
    pub(crate) fn replace_properties(
        &mut self,
        id: NodeId,
        local: &str,
        properties: &str,
        changeable: impl Fn(roxmltree::Node) -> bool,
        reason: &'static str,
    ) -> Result<(), EditError> {
        let scope = self.scope(id);
        let node = self.n(id);
        let (head, tail) = self.tags(self.story_text(node), node);
        let (open, found) = find_properties(&head, &tail, &scope, local).map_err(|e| e.on(id))?;
        let current = found.clone().map_or("", |r| &head[r]);
        let kept = |element: &str| -> Result<Option<Vec<String>>, EditError> {
            if element.is_empty() {
                return Ok(Some(Vec::new()));
            }
            let kept = scope.read(element, "", |props, offset| {
                (w_local(props) == Some(local)).then(|| {
                    xml::elements(props)
                        .filter(|e| !changeable(*e))
                        .map(|e| {
                            element[e.range().start - offset..e.range().end - offset].to_string()
                        })
                        .collect()
                })
            });
            kept.map_err(|e| e.on(id))
        };
        match (kept(current)?, kept(properties)?) {
            (Some(a), Some(b)) if a == b => {}
            _ => return refuse(id, reason),
        }
        let updated = match found {
            Some(range) => format!("{}{properties}{}", &head[..range.start], &head[range.end..]),
            None => format!("{}{properties}{}", &head[..open], &head[open..]),
        };
        let node = self.n(id);
        let unchanged = updated == self.source_tags(self.story_text(node), node).0;
        let node = &mut self.nodes[id.index()];
        match &mut node.made {
            Some(made) => made.head = updated,
            None => {
                // A head back to its source is copied as the source has it,
                // so a self-closing element stays self-closing.
                node.head = (!unchanged).then(|| updated.into_boxed_str());
                node.dirty = !unchanged;
            }
        }
        self.mark_ancestors(id);
        Ok(())
    }
}

fn underlined(props: &RunProps) -> bool {
    props.underline.as_deref().is_some_and(|u| u != "none")
}

/// Replaces the WordprocessingML children named `names` of a properties
/// element with `set`, at their place in the schema's `order`. `scope` is
/// the scope the element is in. A missing element is made, named `name`. An
/// element left with nothing in it is dropped, unless its start tag says
/// something of its own; whatever the start tag holds stays as it is.
pub(crate) fn rewrite(
    element: &str,
    scope: &Scope,
    name: &str,
    order: &[&str],
    names: &[&str],
    mut set: Vec<String>,
) -> Result<String, Unreadable> {
    if element.is_empty() {
        return Ok(match set.is_empty() {
            true => String::new(),
            false => format!("<{name}>{}</{name}>", set.concat()),
        });
    }
    // A properties element can rebind names used by a new child or its attributes.
    let open = tag_end(element, 0).unwrap_or(0);
    if element[..open].contains("xmlns") {
        let inside = scope.within(&element[..open]);
        for child in &mut set {
            *child = scope.carried_to(&inside, child);
        }
    }
    scope.read(element, "", |props, offset| {
        let Some(inner) = inner(element) else {
            if set.is_empty() {
                return element.to_string();
            }
            let start = element.strip_suffix("/>").unwrap_or(element);
            return format!("{}>{}</{}>", start.trim_end(), set.concat(), qname(element));
        };
        let children: Vec<(Range<usize>, Option<&str>)> = xml::elements(props)
            .map(|c| (c.range().start - offset..c.range().end - offset, w_local(c)))
            .collect();
        let ours = |local: &Option<&str>| local.is_some_and(|local| names.contains(&local));
        let rank = order
            .iter()
            .position(|n| *n == names[0])
            .expect("a known property");
        // The first other child that the schema puts after this property.
        let at = children
            .iter()
            .filter(|(_, local)| !ours(local))
            .find(|(_, local)| match local {
                Some(local) => order
                    .iter()
                    .position(|n| n == local)
                    .is_none_or(|p| p > rank),
                None => true,
            })
            .map_or(inner.end, |(r, _)| r.start);
        let mut kept = String::with_capacity(element.len() + 32);
        let mut last = inner.start;
        let mut placed = false;
        for (r, _) in children.iter().filter(|(_, local)| ours(local)) {
            if !placed && at <= r.start {
                kept.push_str(&element[last..at]);
                kept.push_str(&set.concat());
                (last, placed) = (at, true);
            }
            kept.push_str(&element[last..r.start]);
            last = r.end;
        }
        if !placed {
            kept.push_str(&element[last..at]);
            kept.push_str(&set.concat());
            last = at;
        }
        kept.push_str(&element[last..inner.end]);
        if kept.trim().is_empty() && bare(&element[..inner.start]) {
            return String::new();
        }
        format!("{}{kept}{}", &element[..inner.start], &element[inner.end..])
    })
}

/// Whether a start tag holds only its name.
fn bare(tag: &str) -> bool {
    tag[1 + qname(tag).len()..]
        .trim_end_matches(['/', '>'])
        .trim()
        .is_empty()
}

/// A properties element without the WordprocessingML children, at any
/// depth, whose local names are in `locals`. `scope` is the scope the
/// element is in. An element that loses everything it held is dropped; one
/// written with no children stays as it is.
pub(crate) fn without(element: &str, scope: &Scope, locals: &[&str]) -> Result<String, Unreadable> {
    if element.is_empty() {
        return Ok(String::new());
    }
    scope.read(element, "", |props, offset| {
        stripped(element, props, offset, locals)
    })
}

/// `node` of `markup`, as [`without`] leaves it.
fn stripped(markup: &str, node: roxmltree::Node, offset: usize, locals: &[&str]) -> String {
    let element = &markup[node.range().start - offset..node.range().end - offset];
    let start = node.range().start - offset;
    let Some(inner) = inner(element) else {
        return element.to_string();
    };
    let mut children = xml::elements(node).peekable();
    if children.peek().is_none() {
        return element.to_string();
    }
    let mut kept = String::with_capacity(element.len());
    let mut last = start + inner.start;
    let mut any = false;
    for child in children {
        kept.push_str(&markup[last..child.range().start - offset]);
        last = child.range().end - offset;
        if w_local(child).is_some_and(|local| locals.contains(&local)) {
            continue;
        }
        any = true;
        kept.push_str(&stripped(markup, child, offset, locals));
    }
    kept.push_str(&markup[last..start + inner.end]);
    if !any && kept.trim().is_empty() {
        return String::new();
    }
    format!("{}{kept}{}", &element[..inner.start], &element[inner.end..])
}

/// Where a node's start tag ends in its head, and where its WordprocessingML
/// properties element `local` is. `tail` is what follows the node's
/// children, and `scope` the scope inside the node.
fn find_properties(
    head: &str,
    tail: &str,
    scope: &Scope,
    local: &str,
) -> Result<(usize, Option<Range<usize>>), Unreadable> {
    let open = tag_end(head, 0).unwrap_or(head.len());
    // A head that is only a start tag holds no properties.
    if !head[open..].contains('<') {
        return Ok((open, None));
    }
    scope.read(head, tail, |node, offset| {
        let found = xml::elements(node)
            .map(|c| (c.range().start - offset..c.range().end - offset, w_local(c)))
            .find(|(range, name)| *name == Some(local) && range.end <= head.len());
        (open, found.map(|(range, _)| range))
    })
}

/// The content between an element's start and end tags, or `None` when it
/// is self-closing.
pub(crate) fn inner(element: &str) -> Option<Range<usize>> {
    let open = tag_end(element, 0)?;
    if element[..open].ends_with("/>") {
        return None;
    }
    let close = element.rfind("</")?;
    Some(open..close)
}

/// The byte after the `>` that closes the tag starting at `start`, skipping
/// quoted attribute values.
pub(crate) fn tag_end(text: &str, start: usize) -> Option<usize> {
    let mut quote = None;
    for (i, c) in text[start..].char_indices() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '>') => return Some(start + i + 1),
            _ => {}
        }
    }
    None
}

/// The markup of the first WordprocessingML child `local`. `scope` is the
/// scope `element` is in.
pub(crate) fn child<'a>(
    element: &'a str,
    scope: &Scope,
    local: &str,
) -> Result<Option<&'a str>, Unreadable> {
    if element.is_empty() {
        return Ok(None);
    }
    let found = scope.read(element, "", |props, offset| {
        xml::child(props, W, local).map(|c| c.range().start - offset..c.range().end - offset)
    })?;
    Ok(found.map(|range| &element[range]))
}

/// An element's qualified name as its markup spells it.
pub(crate) fn qname(element: &str) -> &str {
    let tag = &element[1..];
    let end = tag
        .find(|c: char| c.is_whitespace() || c == '/' || c == '>')
        .unwrap_or(tag.len());
    &tag[..end]
}

/// Sets the reserved xml:space attribute without touching other attributes.
pub(crate) fn preserve_space(element: &str) -> String {
    let open = tag_end(element, 0).expect("parsed text start tag");
    let value = "preserve";
    let mut at = 1 + qname(element).len();
    while let Some(eq) = element[at..open].find('=') {
        let eq = at + eq;
        let attribute = element[at..eq].trim();
        let after = &element[eq + 1..open];
        let start = eq + 1 + (after.len() - after.trim_start().len());
        let Some(quote) = element[start..open].chars().next() else {
            break;
        };
        let Some(close) = element[start + 1..open].find(quote).map(|c| start + 1 + c) else {
            break;
        };
        if attribute == "xml:space" {
            return format!("{}{value}{}", &element[..start + 1], &element[close..]);
        }
        at = close + 1;
    }
    add_attribute(element, "xml:space", value)
}

pub(crate) fn add_attribute(element: &str, name: &str, value: &str) -> String {
    let open = tag_end(element, 0).expect("parsed start tag");
    let end = match element[..open].ends_with("/>") {
        true => open - 2,
        false => open - 1,
    };
    format!("{} {name}=\"{value}\"{}", &element[..end], &element[end..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_property_after_a_revision_placeholder_is_found_where_it_is() {
        let scope = Scope::default().within(&format!(r#"<w:p xmlns:w="{W}">"#));
        let mark = format!(r#"<w:rPr><w:ins w:id="{REVISION_ID}" w:author="A"/></w:rPr>"#);
        let properties = format!(r#"<w:pPr>{mark}<w:keepNext/><w:jc w:val="center"/></w:pPr>"#);

        let without_keep = rewrite(
            &properties,
            &scope,
            "w:pPr",
            &PPR_ORDER,
            &["keepNext"],
            vec![],
        );
        assert_eq!(
            without_keep.unwrap(),
            format!(r#"<w:pPr>{mark}<w:jc w:val="center"/></w:pPr>"#)
        );
        let aligned = without(&properties, &scope, &["jc"]).unwrap();
        assert_eq!(aligned, format!("<w:pPr>{mark}<w:keepNext/></w:pPr>"));
        assert_eq!(child(&properties, &scope, "rPr").unwrap(), Some(&*mark));

        // A node's head stops before its children, and its tail closes it.
        let head = format!(r#"<w:ins w:id="{REVISION_ID}"><w:rPr><w:b/></w:rPr>"#);
        let found = find_properties(&head, "</w:ins>", &scope, "rPr").unwrap();
        assert_eq!(
            found.1.map(|range| &head[range]),
            Some("<w:rPr><w:b/></w:rPr>")
        );
    }
}
