//! Content controls: what each holds, read from its `w:sdtPr`, and
//! `FillContentControl`, which fills one as a form does.
//!
//! A fill replaces the control's content with one run of the new text, in
//! a paragraph of its own when the control holds paragraphs, and updates
//! the `w:sdtPr`: the placeholder flag goes, and a check box's state, a
//! list's last value or a date picker's date follows the fill.

use roxmltree::Node as X;

use crate::dates::{Date, DEFAULT_PICTURE};
use crate::edit::{check_text, Fill};
use crate::error::EditError;
use crate::format::{self as markup, without};
use crate::import::IN_CONTENT_CONTROL;
use crate::ir::{Control, ListItem, Made, NodeId, NodeKind, ParagraphInfo, RunInfo, Support};
use crate::structure::{fits, refuse};
use crate::xml::{child, elements, escape_attr, is, on_off, w_attr, W, W14};
use crate::Document;

/// Type elements that hold nothing a value fills.
const OTHER: [&str; 9] = [
    "picture",
    "docPartObj",
    "docPartList",
    "group",
    "citation",
    "bibliography",
    "equation",
    "repeatingSection",
    "repeatingSectionItem",
];

/// What a content control holds, and whether it shows its placeholder.
pub(crate) fn control_of(props: Option<X>) -> (Control, bool) {
    let Some(props) = props else {
        return (Control::RichText, false);
    };
    let placeholder = on_off(props, "showingPlcHdr").unwrap_or(false);
    let control = elements(props)
        .find_map(|e| type_of(e))
        .unwrap_or(Control::RichText);
    (control, placeholder)
}

fn type_of(e: X) -> Option<Control> {
    let on = |v: Option<&str>| v.is_some_and(|v| matches!(v, "1" | "true" | "on"));
    if is(e, W, "richText") {
        return Some(Control::RichText);
    }
    if is(e, W, "text") {
        return Some(Control::Text {
            multi_line: on(w_attr(e, "multiLine")),
        });
    }
    if is(e, W14, "checkbox") {
        let symbol = |state: &str, default: char| {
            child(e, W14, state)
                .and_then(|s| s.attribute((W14, "val")))
                .and_then(|v| u32::from_str_radix(v, 16).ok())
                .and_then(char::from_u32)
                .unwrap_or(default)
        };
        return Some(Control::Checkbox {
            checked: on(child(e, W14, "checked").and_then(|c| c.attribute((W14, "val")))),
            checked_symbol: symbol("checkedState", '\u{2612}'),
            unchecked_symbol: symbol("uncheckedState", '\u{2610}'),
        });
    }
    if is(e, W, "dropDownList") || is(e, W, "comboBox") {
        let items = elements(e)
            .filter(|i| is(*i, W, "listItem"))
            .filter_map(|i| {
                let value = w_attr(i, "value");
                let display_text = w_attr(i, "displayText").or(value)?;
                Some(ListItem {
                    display_text: display_text.to_string(),
                    value: value.unwrap_or(display_text).to_string(),
                })
            })
            .collect();
        let last_value = w_attr(e, "lastValue").map(str::to_string);
        return Some(match is(e, W, "comboBox") {
            true => Control::ComboBox { items, last_value },
            false => Control::DropDown { items, last_value },
        });
    }
    if is(e, W, "date") {
        return Some(Control::Date {
            full_date: w_attr(e, "fullDate").map(str::to_string),
            format: child(e, W, "dateFormat")
                .and_then(|f| w_attr(f, "val"))
                .map(str::to_string),
        });
    }
    let local = e.tag_name().name();
    OTHER.contains(&local).then(|| Control::Other {
        element: local.to_string(),
    })
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct ControlValue {
    text: String,
    properties: String,
    run_properties: String,
}

impl Document {
    pub(crate) fn prepare_control_fill(
        &self,
        id: NodeId,
        fill: Fill,
    ) -> Result<ControlValue, EditError> {
        let node = self.attached(id)?;
        let NodeKind::ContentControl {
            data_bound,
            lock,
            control,
            placeholder,
            ..
        } = node.kind.clone()
        else {
            return Err(EditError::WrongKind {
                node: id,
                kind: node.kind.name(),
                expected: "a content control",
            });
        };
        self.check_editable(self.parent_of(id)?)?;
        if data_bound {
            return refuse(
                id,
                "it is bound to custom XML, which a fill would leave stale",
            );
        }
        if matches!(lock.as_deref(), Some("contentLocked" | "sdtContentLocked")) {
            return refuse(id, "it is locked against editing");
        }
        if self.n(id).children.is_empty() {
            return refuse(id, "it holds no content to replace");
        }
        if let Some(reason) = self.content_refusal(id) {
            return refuse(id, reason);
        }
        let current = self.properties(id, "sdtPr");
        let scope = self.scope(id);
        let cleared = without(&current, &scope, &["showingPlcHdr"]);
        let property = |ns, types: &[&str], child_name, attr, value: &str| {
            edit_property(&scope, &cleared, ns, types, child_name, attr, value).ok_or(
                EditError::Refused {
                    node: id,
                    reason: "the control properties could not be read",
                },
            )
        };
        let (text, properties) = match (control, fill) {
            (Control::Text { multi_line }, Fill::Text(text)) => {
                check_fill_text(id, &text, multi_line)?;
                (text, cleared)
            }
            (Control::RichText, Fill::Text(text)) => {
                check_fill_text(id, &text, true)?;
                (text, cleared)
            }
            (
                Control::Checkbox {
                    checked,
                    checked_symbol,
                    unchecked_symbol,
                },
                Fill::Checked(on),
            ) => {
                if checked == on {
                    let reason = match on {
                        true => "it is checked already",
                        false => "it is unchecked already",
                    };
                    return refuse(id, reason);
                }
                let symbol = if on { checked_symbol } else { unchecked_symbol };
                let state = if on { "1" } else { "0" };
                let properties = property(W14, &["checkbox"], Some("checked"), "val", state)?;
                (symbol.to_string(), properties)
            }
            (
                Control::DropDown { items, .. } | Control::ComboBox { items, .. },
                Fill::Item(value),
            ) => {
                let Some(item) = items.iter().find(|i| i.value == value) else {
                    return refuse(id, "no list item has that value");
                };
                let properties =
                    property(W, &["dropDownList", "comboBox"], None, "lastValue", &value)?;
                (item.display_text.clone(), properties)
            }
            (Control::Date { format, .. }, Fill::Date(value)) => {
                let Some(date) = Date::parse(&value) else {
                    return refuse(id, "the date is not a real date written YYYY-MM-DD");
                };
                let properties = property(W, &["date"], None, "fullDate", &date.full())?;
                let shown = date.format(format.as_deref().unwrap_or(DEFAULT_PICTURE));
                (shown, properties)
            }
            _ => return refuse(id, "the control takes another kind of value"),
        };
        let run_properties = self.control_run(id, placeholder, &current);
        Ok(ControlValue {
            text,
            properties,
            run_properties,
        })
    }

    /// Materialize a validated value. No user-edit policy runs on a merged value.
    pub(crate) fn compose_control(
        &mut self,
        id: NodeId,
        value: &ControlValue,
    ) -> Result<(), EditError> {
        // A structural removal detaches the control itself. Its stored fill
        // becomes visible again on undo. A paragraph containing the control
        // can also be temporarily detached while a join is materialized.
        let control = self.node(id).ok_or(EditError::UnknownNode { node: id })?;
        if !matches!(control.kind, NodeKind::ContentControl { .. }) {
            return Err(EditError::WrongKind {
                node: id,
                kind: control.kind.name(),
                expected: "a content control",
            });
        }
        let Some(parent) = control.parent else {
            return Ok(());
        };
        let content = self.filled_content(id, parent, value)?;
        for child in self.n(id).children.clone() {
            self.detach(child);
        }
        self.attach(content, id, 0);
        self.compose_control_properties(id, &value.properties)?;
        Ok(())
    }

    /// Why a fill may not replace what a control holds: anything but text,
    /// tabs, breaks and proofing marks, which it would lose.
    fn content_refusal(&self, id: NodeId) -> Option<&'static str> {
        const TRACKED: &str = "it holds a tracked change; accept or reject it first";
        for inside in self.descendants(id) {
            if self.n(inside).support == Support::ReadOnly(crate::import::MIXED_TEXT) {
                return Some("it contains embedded XML markup, which a fill would lose");
            }
            let reason = match &self.n(inside).kind {
                NodeKind::Paragraph(p) if p.mark_revision.is_some() || p.format_revision => TRACKED,
                NodeKind::Paragraph(p) if p.section.is_some() => "it holds a section break",
                NodeKind::Run(r) if r.format_revision => TRACKED,
                NodeKind::Revision(_) | NodeKind::DeletedText { .. } => TRACKED,
                NodeKind::Paragraph(_)
                | NodeKind::Run(_)
                | NodeKind::Text { .. }
                | NodeKind::Tab
                | NodeKind::Break { .. } => continue,
                NodeKind::Marker { element, .. }
                    if matches!(local(element), "proofErr" | "lastRenderedPageBreak") =>
                {
                    continue
                }
                _ => "it holds more than text, which a fill would lose",
            };
            return Some(reason);
        }
        None
    }

    /// The properties of the run a fill makes, readable where the control
    /// is: those the control's properties give while the placeholder shows,
    /// and else those of its first run.
    fn control_run(&self, id: NodeId, placeholder: bool, properties: &str) -> String {
        let scope = self.scope(id);
        let first_run = self
            .descendants(id)
            .into_iter()
            .find(|&r| matches!(self.n(r).kind, NodeKind::Run(_)));
        match (placeholder, first_run) {
            (false, Some(run)) => self
                .scope(run)
                .carried_to(&scope, &self.properties(run, "rPr")),
            _ => {
                let rpr = markup::child(properties, &scope, "rPr").unwrap_or_default();
                let open = markup::tag_end(properties, 0).unwrap_or(0);
                scope.within(&properties[..open]).carried_to(&scope, rpr)
            }
        }
    }

    fn filled_content(
        &mut self,
        id: NodeId,
        parent: NodeId,
        value: &ControlValue,
    ) -> Result<NodeId, EditError> {
        let story = self.n(id).story;
        let rpr = &value.run_properties;
        let (style, props) = self.run_properties_from(id, rpr)?;
        let info = RunInfo {
            style,
            props,
            ..RunInfo::default()
        };
        let text = &value.text;
        let read_only = Support::ReadOnly(IN_CONTENT_CONTROL);
        let made = |name: String, head: String| Made {
            head,
            tail: format!("</{name}>"),
            like: None,
        };
        let (r, t, br) = (
            self.w_name(id, "r"),
            self.w_name(id, "t"),
            self.w_name(id, "br"),
        );
        let run_made = made(r.clone(), format!("<{r}>{rpr}"));
        let run = self.create(NodeKind::Run(Box::new(info)), read_only, run_made, story);
        for (i, line) in text.split('\n').enumerate() {
            if i > 0 {
                let made = Made {
                    head: format!("<{br}/>"),
                    tail: String::new(),
                    like: None,
                };
                let kind = NodeKind::Break { break_type: None };
                let node = self.create(kind, read_only, made, story);
                self.adopt(run, node);
            }
            if !line.is_empty() {
                let made = made(t.clone(), format!("<{t} xml:space=\"preserve\">"));
                let kind = NodeKind::Text {
                    text: line.to_string(),
                };
                let node = self.create(kind, read_only, made, story);
                self.adopt(run, node);
            }
        }
        let paragraph = NodeKind::Paragraph(Box::<ParagraphInfo>::default());
        if !fits(&paragraph, &self.n(parent).kind) {
            return Ok(run);
        }
        let like = self
            .n(id)
            .children
            .iter()
            .copied()
            .find(|&c| matches!(self.n(c).kind, NodeKind::Paragraph(_)));
        let paragraph = self.empty_paragraph(like, id);
        self.nodes[paragraph.index()].support = read_only;
        self.adopt(paragraph, run);
        Ok(paragraph)
    }

    fn compose_control_properties(
        &mut self,
        id: NodeId,
        properties: &str,
    ) -> Result<String, EditError> {
        let (control, placeholder) = self
            .read_markup(id, properties, |p| control_of(Some(p)))?
            .unwrap_or((Control::RichText, false));
        let current = self.replace_properties(
            id,
            "sdtPr",
            properties,
            |scope, e| {
                matches!(
                    scope.local(e),
                    Some("showingPlcHdr" | "dropDownList" | "comboBox" | "date")
                ) || scope.local_in(e, W14) == Some("checkbox")
            },
            "only the placeholder flag and a check box, list or date can change this way",
        )?;
        if let NodeKind::ContentControl {
            control: c,
            placeholder: p,
            ..
        } = &mut self.nodes[id.index()].kind
        {
            (*c, *p) = (control, placeholder);
        }
        Ok(current)
    }
}

/// Change only the selected OOXML attribute. Use the parsed byte range so
/// extension elements and attributes with the same local names survive.
fn edit_property(
    scope: &markup::Scope,
    properties: &str,
    ns: &str,
    types: &[&str],
    child_name: Option<&str>,
    attr: &str,
    value: &str,
) -> Option<String> {
    let head = format!("<scope{}>", scope.declarations());
    let wrapped = format!("{head}{properties}</scope>");
    let doc = crate::xml::parse("content control", &wrapped).ok()?;
    let props = doc.root_element().first_element_child()?;
    let kind = elements(props).find(|e| types.iter().any(|name| is(*e, ns, name)))?;
    let target = match child_name {
        Some(name) => child(kind, ns, name),
        None => Some(kind),
    };
    let mut edited = properties.to_owned();
    if let Some(attribute) = target.and_then(|n| n.attribute_node((ns, attr))) {
        let range = attribute.range_value();
        // Keep the source's quote style when replacing just the value.
        let mut escaped = escape_attr(value);
        if wrapped.as_bytes()[range.start - 1] == b'\'' {
            escaped = escaped.replace('\'', "&apos;");
        }
        edited.replace_range(range.start - head.len()..range.end - head.len(), &escaped);
    } else {
        let node = target.unwrap_or(kind);
        let range = node.range();
        let source = &wrapped[range.clone()];
        let name = markup::Scope::from_element(node).attribute(
            ns,
            if ns == W14 { "w14" } else { "w" },
            attr,
        );
        let updated = match (target, child_name) {
            (None, Some(child)) => first_child(
                source,
                &format!(
                    "<{}{child} {name}=\"{}\"/>",
                    prefix_of(source),
                    escape_attr(value)
                ),
            ),
            _ => markup::add_attribute(source, &name, &escape_attr(value)),
        };
        edited.replace_range(range.start - head.len()..range.end - head.len(), &updated);
    }
    Some(edited)
}

/// Refuses text a control could not hold: none, a tab, a carriage return,
/// what XML cannot represent, or a line feed where only one line fits.
fn check_fill_text(id: NodeId, text: &str, multi_line: bool) -> Result<(), EditError> {
    if text.is_empty() {
        return refuse(id, "a text fill needs text");
    }
    if !multi_line && text.contains('\n') {
        return refuse(id, "the control holds one line");
    }
    text.split('\n').try_for_each(check_text)
}

fn local(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

/// The prefix, with its colon, an element's markup writes its name with.
fn prefix_of(element: &str) -> &str {
    let name = markup::qname(element);
    &name[..name.len() - local(name).len()]
}

/// `element` with `child` as its first child.
fn first_child(element: &str, child: &str) -> String {
    let open = markup::tag_end(element, 0).unwrap_or(element.len());
    match element[..open].strip_suffix("/>") {
        Some(start) => {
            let name = markup::qname(element);
            format!("{}>{child}</{name}>", start.trim_end())
        }
        None => format!("{}{child}{}", &element[..open], &element[open..]),
    }
}
