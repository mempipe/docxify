//! Run and paragraph properties are edited as markup so that unrelated
//! properties retain their source bytes.

use std::ops::Range;

use crate::edit::{Format, Point};
use crate::error::EditError;
use crate::ir::{NodeId, NodeKind, ParaProps, RunInfo, RunProps};
use crate::structure::refuse;
use crate::xml::{escape_attr, W};
use crate::Document;

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

    pub(crate) fn declarations(&self) -> String {
        self.0
            .iter()
            .map(|(prefix, value)| {
                if prefix.is_empty() {
                    format!(" xmlns={value}")
                } else {
                    format!(" xmlns:{prefix}={value}")
                }
            })
            .collect()
    }

    /// A qualified attribute, with a local declaration if no usable prefix is in scope.
    pub(crate) fn attribute(&self, ns: &str, preferred: &str, local: &str) -> String {
        if let Some(prefix) = self
            .0
            .keys()
            .find(|p| !p.is_empty() && self.namespace(p) == Some(ns))
        {
            return format!("{prefix}:{local}");
        }
        let mut prefix = preferred.to_owned();
        while self.0.contains_key(&prefix) {
            prefix.push('_');
        }
        format!("xmlns:{prefix}=\"{ns}\" {prefix}:{local}")
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

    pub(crate) fn local<'a>(&self, markup: &'a str) -> Option<&'a str> {
        self.local_in(markup, W)
    }

    pub(crate) fn local_in<'a>(&self, markup: &'a str, ns: &str) -> Option<&'a str> {
        let name = qname(markup);
        let (prefix, local) = name.split_once(':').unwrap_or(("", name));
        let tag = &markup[..tag_end(markup, 0)?];
        let named = match tag.contains("xmlns") {
            true => self.within(tag).namespace(prefix) == Some(ns),
            false => self.namespace(prefix) == Some(ns),
        };
        named.then_some(local)
    }

    pub(crate) fn is(&self, markup: &str, local: &str) -> bool {
        self.local(markup) == Some(local)
    }

    /// Bind a detached copy even if its eventual parent changes the default
    /// namespace or shadows a prefix inherited from the story.
    pub(crate) fn detached_declarations(&self) -> String {
        let mut bindings = self.declarations();
        if !self.0.contains_key("") {
            bindings.push_str(" xmlns=\"\"");
        }
        bindings
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
        }
        scope
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
        let mut wrapped = format!("<scope{}>{markup}</scope>", self.scope(id).declarations());
        // Copied revisions acquire their actual IDs at export. Formatting
        // readers need valid XML, and none of them consumes a revision ID.
        if markup.contains(crate::revisions::REVISION_ID) {
            wrapped = wrapped.replace(crate::revisions::REVISION_ID, "0");
        }
        let invalid = || EditError::Refused {
            node: id,
            reason: "the properties are not well-formed markup",
        };
        let doc = crate::xml::parse("properties", &wrapped).map_err(|_| invalid())?;
        let mut roots = crate::xml::elements(doc.root_element());
        let root = roots.next().ok_or_else(invalid)?;
        if roots.next().is_some() {
            return Err(invalid());
        }
        Ok(Some(read(root)))
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

    pub(crate) fn styled(&self, paragraph: NodeId, style: &str) -> String {
        let default = self.styles.default_paragraph_style().map(|s| s.id.as_str());
        let name = self.w_name(paragraph, "pPr");
        let set = (Some(style) != default).then(|| {
            let element = self.w_name(paragraph, "pStyle");
            let val = self.w_attr_name(paragraph, "val");
            vec![format!("<{element} {val}=\"{}\"/>", escape_attr(style))]
        });
        rewrite(
            &self.properties(paragraph, "pPr"),
            &self.scope(paragraph),
            &name,
            &PPR_ORDER,
            &STYLE,
            set.unwrap_or_default(),
        )
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
    pub(crate) fn formatted(&self, run: NodeId, change: Format) -> String {
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
        let mut rpr = self.properties(run, "rPr");
        if let Some(on) = change.bold {
            let set = (on != styled.bold.unwrap_or(false)).then(|| toggle(on, &BOLD));
            rpr = rewrite(&rpr, scope, &name, &ORDER, &BOLD, set.unwrap_or_default());
        }
        if let Some(on) = change.italic {
            let set = (on != styled.italic.unwrap_or(false)).then(|| toggle(on, &ITALIC));
            rpr = rewrite(&rpr, scope, &name, &ORDER, &ITALIC, set.unwrap_or_default());
        }
        if let Some(on) = change.underline {
            let value = if on { "single" } else { "none" };
            let set = (on != underlined(&styled))
                .then(|| vec![format!("<{prefix}u {val}=\"{value}\"/>")]);
            rpr = rewrite(
                &rpr,
                scope,
                &name,
                &ORDER,
                &UNDERLINE,
                set.unwrap_or_default(),
            );
        }
        rpr
    }

    /// The run's `w:rPr` with the character style `style`, or with none.
    pub(crate) fn run_styled(&self, run: NodeId, style: Option<&str>) -> String {
        let name = self.w_name(run, "rPr");
        let set = style.map(|style| {
            let element = self.w_name(run, "rStyle");
            let val = self.w_attr_name(run, "val");
            vec![format!("<{element} {val}=\"{}\"/>", escape_attr(style))]
        });
        let current = self.properties(run, "rPr");
        rewrite(
            &current,
            &self.scope(run),
            &name,
            &ORDER,
            &RUN_STYLE,
            set.unwrap_or_default(),
        )
    }

    /// A node's properties element, `w:rPr` or `w:pPr`, as markup, or
    /// nothing.
    pub(crate) fn properties(&self, id: NodeId, local: &str) -> String {
        let head = self.head(id);
        find_properties(&head, &self.scope(id), local)
            .1
            .map_or(String::new(), |range| head[range].to_string())
    }

    /// A source node's properties element as its source has it, whatever
    /// commands changed since.
    pub(crate) fn source_properties(&self, id: NodeId, local: &str) -> String {
        let node = self.n(id);
        let head = self.source_tags(self.story_text(node), node).0;
        find_properties(&head, &self.scope(id), local)
            .1
            .map_or(String::new(), |range| head[range].to_string())
    }

    /// Writes final run properties while a builder is assembling a tree.
    /// Unlike a user step, this also takes a detached run.
    pub(crate) fn compose_run_properties(
        &mut self,
        id: NodeId,
        properties: &str,
    ) -> Result<String, EditError> {
        let changeable: Vec<&str> = BOLD
            .iter()
            .chain(&ITALIC)
            .chain(&UNDERLINE)
            .chain(&RUN_STYLE)
            .copied()
            .collect();
        let parsed = self.run_properties_from(id, properties)?;
        let current = self.replace_properties(
            id,
            "rPr",
            properties,
            |scope, e| scope.local(e).is_some_and(|n| changeable.contains(&n)),
            "only bold, italic, underline and the character style can change this way",
        )?;
        if let NodeKind::Run(info) = &mut self.nodes[id.index()].kind {
            (info.style, info.props) = parsed;
        }
        Ok(current)
    }

    /// Writes final paragraph properties before or after arrangement.
    pub(crate) fn compose_paragraph_properties(
        &mut self,
        id: NodeId,
        properties: &str,
    ) -> Result<String, EditError> {
        let parsed = self.paragraph_properties_from(id, properties)?;
        let current = self.replace_properties(
            id,
            "pPr",
            properties,
            |scope, e| scope.local(e).is_some_and(|n| PARAGRAPH.contains(&n)),
            "only the paragraph style and numbering can change this way",
        )?;
        if let NodeKind::Paragraph(info) = &mut self.nodes[id.index()].kind {
            (info.style, info.props) = parsed;
        }
        Ok(current)
    }

    /// Replaces a properties element if the remaining children match after
    /// filtering the permitted changes in each child's namespace scope.
    pub(crate) fn replace_properties(
        &mut self,
        id: NodeId,
        local: &str,
        properties: &str,
        changeable: impl Fn(&Scope, &str) -> bool,
        reason: &'static str,
    ) -> Result<String, EditError> {
        let scope = self.scope(id);
        let head = self.head(id);
        let (open, found) = find_properties(&head, &scope, local);
        let current = found.clone().map_or("", |r| &head[r]).to_string();
        let kept = |element: &str| -> Option<Vec<String>> {
            if element.is_empty() {
                return Some(Vec::new());
            }
            if !scope.is(element, local) {
                return None;
            }
            let Some(inner) = inner(element) else {
                return Some(Vec::new());
            };
            let scope = scope.within(&element[..inner.start]);
            Some(
                elements(&element[inner.clone()])
                    .into_iter()
                    .map(|r| element[inner.start + r.start..inner.start + r.end].to_string())
                    .filter(|e| !changeable(&scope, e))
                    .collect(),
            )
        };
        match (kept(&current), kept(properties)) {
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
        Ok(current)
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
    set: Vec<String>,
) -> String {
    // A properties element can shadow the prefix used to generate its new
    // Word children. Bind those children before splicing them into its body.
    let open = tag_end(element, 0).unwrap_or(0);
    let inside = scope.within(&element[..open]);
    let mut set = set;
    if element[..open].contains("xmlns") {
        for child in &mut set {
            if inside.local(child).is_none() && scope.local(child).is_some() {
                *child = scope.carried_to(&inside, child);
            }
        }
    }
    let Some(inner) = inner(element).filter(|_| !element.is_empty()) else {
        if set.is_empty() {
            return element.to_string();
        }
        return match element.strip_suffix("/>") {
            Some(start) => format!("{}>{}</{}>", start.trim_end(), set.concat(), qname(element)),
            None => format!("<{name}>{}</{name}>", set.concat()),
        };
    };
    let scope = &inside;
    let body = &element[inner.clone()];
    let children = elements(body);
    let ours = |r: &Range<usize>| {
        scope
            .local(&body[r.clone()])
            .is_some_and(|local| names.contains(&local))
    };
    let rank = order
        .iter()
        .position(|n| *n == names[0])
        .expect("a known property");
    // The first other child that the schema puts after this property.
    let at = children
        .iter()
        .filter(|r| !ours(r))
        .find(|r| match scope.local(&body[(*r).clone()]) {
            Some(local) => order
                .iter()
                .position(|n| *n == local)
                .is_none_or(|p| p > rank),
            None => true,
        })
        .map_or(body.len(), |r| r.start);
    let mut kept = String::with_capacity(body.len() + 32);
    let mut last = 0;
    let mut placed = false;
    for r in children.iter().filter(|r| ours(r)) {
        if !placed && at <= r.start {
            kept.push_str(&body[last..at]);
            kept.push_str(&set.concat());
            (last, placed) = (at, true);
        }
        kept.push_str(&body[last..r.start]);
        last = r.end;
    }
    if !placed {
        kept.push_str(&body[last..at]);
        kept.push_str(&set.concat());
        last = at;
    }
    kept.push_str(&body[last..]);
    if kept.trim().is_empty() && bare(&element[..inner.start]) {
        return String::new();
    }
    format!("{}{kept}{}", &element[..inner.start], &element[inner.end..])
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
pub(crate) fn without(element: &str, scope: &Scope, locals: &[&str]) -> String {
    let Some(inner) = inner(element).filter(|_| !element.is_empty()) else {
        return element.to_string();
    };
    let body = &element[inner.clone()];
    let children = elements(body);
    if children.is_empty() {
        return element.to_string();
    }
    let scope = &scope.within(&element[..inner.start]);
    let mut kept = String::with_capacity(body.len());
    let mut last = 0;
    let mut any = false;
    for r in children {
        kept.push_str(&body[last..r.start]);
        last = r.end;
        let child = &body[r];
        if scope
            .local(child)
            .is_some_and(|local| locals.contains(&local))
        {
            continue;
        }
        any = true;
        kept.push_str(&without(child, scope, locals));
    }
    kept.push_str(&body[last..]);
    if !any && kept.trim().is_empty() {
        return String::new();
    }
    format!("{}{kept}{}", &element[..inner.start], &element[inner.end..])
}

/// Where a node's start tag ends in its head, and where its WordprocessingML
/// properties element `local` is. `scope` is the scope inside the node.
fn find_properties(head: &str, scope: &Scope, local: &str) -> (usize, Option<Range<usize>>) {
    let open = tag_end(head, 0).unwrap_or(head.len());
    let found = elements(&head[open..])
        .into_iter()
        .find(|r| scope.is(&head[open + r.start..open + r.end], local))
        .map(|r| open + r.start..open + r.end);
    (open, found)
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

/// The top-level elements in a stretch of markup.
pub(crate) fn elements(body: &str) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let (mut depth, mut start, mut at) = (0usize, 0, 0);
    while let Some(found) = body[at..].find('<') {
        let i = at + found;
        if body[i..].starts_with("<!--") {
            at = body[i..].find("-->").map_or(body.len(), |e| i + e + 3);
            continue;
        }
        if body[i..].starts_with("<?") {
            at = body[i..].find("?>").map_or(body.len(), |e| i + e + 2);
            continue;
        }
        let Some(end) = tag_end(body, i) else {
            break;
        };
        if body[i..].starts_with("</") {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                out.push(start..end);
            }
        } else if body[..end].ends_with("/>") {
            if depth == 0 {
                out.push(i..end);
            }
        } else {
            if depth == 0 {
                start = i;
            }
            depth += 1;
        }
        at = end;
    }
    out
}

/// The markup of the first WordprocessingML child `local`. `scope` is the
/// scope `element` is in.
pub(crate) fn child<'a>(element: &'a str, scope: &Scope, local: &str) -> Option<&'a str> {
    let inner = inner(element).filter(|_| !element.is_empty())?;
    let scope = scope.within(&element[..inner.start]);
    elements(&element[inner.clone()])
        .into_iter()
        .map(|r| &element[inner.start + r.start..inner.start + r.end])
        .find(|e| scope.is(e, local))
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
