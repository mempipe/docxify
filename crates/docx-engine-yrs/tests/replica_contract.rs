//! R4 rows 3–7, 11 and 15 through shipped command preparation, Yrs storage,
//! materialization and DOCX export. The lower-store contract is complementary.
#[path = "../../docx-engine/tests/common/mod.rs"]
mod common;
use common::*;
use docx_engine::{Command, Document, Format, NodeKind, Point};
use docx_engine_yrs::Replica;

// Formatting bases are observable here: R1 is italic, R2 is underlined.
fn seen(d: &Document) -> Vec<(char, bool, bool, bool)> {
    d.paragraphs()
        .into_iter()
        .flat_map(|p| d.descendants(p))
        .flat_map(|id| {
            let n = d.node(id).unwrap();
            let NodeKind::Text { text } = &n.kind else {
                return vec![];
            };
            let NodeKind::Run(run) = &d.node(n.parent.unwrap()).unwrap().kind else {
                panic!("text has no run")
            };
            text.chars()
                .map(|c| {
                    (
                        c,
                        run.props.bold.unwrap_or(false),
                        run.props.italic.unwrap_or(false),
                        run.props.underline.as_deref().is_some_and(|v| v != "none"),
                    )
                })
                .collect::<Vec<_>>()
        })
        .collect()
}
fn pos(d: &Document, mut offset: usize) -> Point {
    for p in d.paragraphs() {
        for id in d.descendants(p) {
            if let NodeKind::Text { text } = &d.node(id).unwrap().kind {
                let len = text.chars().count();
                if offset <= len {
                    return at(id, offset);
                }
                offset -= len;
            }
        }
    }
    panic!("position outside text")
}
fn delete(r: &mut Replica, start: usize, end: usize) {
    let d = r.view().unwrap();
    let cmd = Command::ReplaceRange {
        from: pos(d, start),
        to: pos(d, end),
        text: String::new(),
        track: None,
    };
    r.apply(&cmd).unwrap();
}
fn insert(r: &mut Replica, offset: usize, text: &str) {
    let at = pos(r.view().unwrap(), offset);
    r.apply(&Command::InsertText {
        at,
        text: text.into(),
    })
    .unwrap();
}
fn bold(r: &mut Replica, start: usize, end: usize, on: bool) {
    let d = r.view().unwrap();
    let cmd = Command::FormatRange {
        from: pos(d, start),
        to: pos(d, end),
        format: Format {
            bold: Some(on),
            ..Format::default()
        },
    };
    r.apply(&cmd).unwrap();
}
fn sync(a: &mut Replica, b: &mut Replica, reverse: bool) {
    let ua = a.update_since(&b.state_vector()).unwrap();
    let ub = b.update_since(&a.state_vector()).unwrap();
    if reverse {
        b.apply_update(&ua).unwrap();
        a.apply_update(&ub).unwrap();
    } else {
        a.apply_update(&ub).unwrap();
        b.apply_update(&ua).unwrap();
    }
    // Include duplicate delivery and check both the displayed tree and export.
    a.apply_update(&ub).unwrap();
    b.apply_update(&ua).unwrap();
    let expected = seen(a.view().unwrap());
    assert_eq!(expected, seen(b.view().unwrap()));
    for r in [a, b] {
        let exported = r.export().unwrap();
        assert_valid("contract", &exported);
        assert_eq!(expected, seen(&Document::open(&exported).unwrap()));
    }
}
fn replicas(body: &str) -> (Vec<u8>, Replica, Replica) {
    let bytes = with_body(body);
    let a = Replica::seed(1, &bytes).unwrap();
    let b = Replica::join(2, &bytes, &a.save()).unwrap();
    (bytes, a, b)
}
fn plain(text: &str) -> String {
    format!("<w:p><w:r><w:t>{text}</w:t></w:r></w:p>")
}

#[test]
fn row_3_two_deletions_do_not_change_untouched_basis() {
    for reverse in [false, true] {
        let (_, mut a, mut b) = replicas(
            r#"<w:p><w:r><w:rPr><w:i/></w:rPr><w:t>AB</w:t></w:r><w:r><w:rPr><w:u w:val="single"/></w:rPr><w:t>C</w:t></w:r><w:r><w:rPr><w:i/></w:rPr><w:t>Q</w:t></w:r><w:r><w:rPr><w:u w:val="single"/></w:rPr><w:t>D</w:t></w:r></w:p>"#,
        );
        delete(&mut a, 1, 3);
        delete(&mut b, 3, 4);
        sync(&mut a, &mut b, reverse);
        assert_eq!(
            seen(a.view().unwrap()),
            vec![('A', false, true, false), ('D', false, false, true)]
        );
    }
}
#[test]
fn rows_4_and_5_overlapping_format_commands_resolve_per_character() {
    for off in [false, true] {
        for reverse in [false, true] {
            let (_, mut a, mut b) = replicas(&plain("abcde"));
            if off {
                bold(&mut a, 1, 4, true);
                bold(&mut b, 2, 4, false);
            } else {
                bold(&mut a, 1, 3, true);
                bold(&mut b, 2, 5, true);
            }
            sync(&mut a, &mut b, reverse);
            let got = seen(a.view().unwrap());
            assert_eq!(got.iter().map(|x| x.0).collect::<String>(), "abcde");
            assert!(!got[0].1);
            assert!(got[1].1);
            if off {
                assert_eq!(got[2].1, got[3].1);
                assert!(!got[4].1);
            } else {
                assert!(got[2..].iter().all(|x| x.1));
            }
            assert!(got.iter().all(|x| !x.2 && !x.3));
        }
    }
}
#[test]
fn rows_6_and_7_typing_inside_and_at_edge_of_concurrent_bold() {
    for edge in [false, true] {
        for reverse in [false, true] {
            let (_, mut a, mut b) = replicas(&plain(if edge { "abc" } else { "abcde" }));
            bold(&mut a, 1, if edge { 3 } else { 4 }, true);
            insert(&mut b, 3, "Q");
            sync(&mut a, &mut b, reverse);
            let got = seen(a.view().unwrap());
            assert_eq!(
                got.iter().map(|x| x.0).collect::<String>(),
                if edge { "abcQ" } else { "abcQde" }
            );
            if !edge {
                assert!(got[3].1, "interior typing must be bold");
            }
            assert!(got[1].1 && got[2].1);
            assert!(got.iter().all(|x| !x.2 && !x.3));
        }
    }
}
#[test]
fn row_11_undo_deletion_preserves_concurrent_typing_and_basis() {
    for reverse in [false, true] {
        let (bytes, mut a, mut b) =
            replicas("<w:p><w:r><w:rPr><w:i/></w:rPr><w:t>abcde</w:t></w:r></w:p>");
        delete(&mut a, 1, 4);
        insert(&mut b, 3, "Z");
        sync(&mut a, &mut b, reverse);
        a.undo().unwrap();
        sync(&mut a, &mut b, reverse);
        let expected = "abcZde"
            .chars()
            .map(|c| (c, false, true, false))
            .collect::<Vec<_>>();
        assert_eq!(seen(a.view().unwrap()), expected);
        let mut reopened = Replica::join(3, &bytes, &a.save()).unwrap();
        assert_eq!(seen(reopened.view().unwrap()), expected);
    }
}

#[test]
fn row_15_typing_at_overlapping_bold_edges_preserves_the_surviving_range() {
    let (bytes, mut ada, mut ben) = replicas(&plain("AB"));
    bold(&mut ada, 0, 1, true);
    sync(&mut ada, &mut ben, false);
    bold(&mut ben, 0, 2, true);
    let mut cara = Replica::join(3, &bytes, &ben.save()).unwrap();
    insert(&mut cara, 1, "X");
    sync(&mut cara, &mut ben, true);
    assert!(ben.undo().unwrap());
    sync(&mut ben, &mut ada, false);
    sync(&mut ben, &mut cara, true);
    assert_eq!(
        seen(cara.view().unwrap()),
        [
            ('A', true, false, false),
            ('X', true, false, false),
            ('B', false, false, false)
        ]
    );
}

#[test]
fn consecutive_keys_write_sibling_generations() {
    let (_, mut r, _) = replicas(&plain("A"));
    for offset in 1..=32 {
        insert(&mut r, offset, "x");
    }
    let store = docx_engine_yrs::store::Store::load(2, &r.save()).unwrap();
    let typed: Vec<_> = store
        .read()
        .unwrap()
        .into_iter()
        .filter(|unit| unit.content == docx_engine::flow::Content::Char('x'))
        .collect();
    assert_eq!(typed.len(), 32);
    let mut generations = std::collections::HashSet::new();
    for unit in &typed {
        let births = unit.all("birth");
        assert_eq!(births.len(), 1);
        assert!(generations.insert(births[0]));
    }
}

#[test]
fn formatting_a_selection_holds_over_what_another_formatted_first_and_then_undoes() {
    let (_, mut ada, mut ben) = replicas(&plain("one two three"));
    bold(&mut ada, 4, 7, true);
    sync(&mut ada, &mut ben, false);
    bold(&mut ben, 0, 13, true);
    sync(&mut ada, &mut ben, true);
    assert!(ada.undo().unwrap());
    sync(&mut ada, &mut ben, false);
    assert!(
        seen(ben.view().unwrap()).iter().all(|(_, bold, ..)| *bold),
        "{:?}",
        seen(ben.view().unwrap())
    );
}

#[test]
fn a_paragraph_style_holds_over_a_paragraph_another_styled_first_and_then_undoes() {
    let bytes = generated("headings-styles");
    let mut ada = Replica::seed(1, &bytes).unwrap();
    let mut ben = Replica::join(2, &bytes, &ada.save()).unwrap();
    let style = |d: &Document, p| match &d.node(p).unwrap().kind {
        NodeKind::Paragraph(info) => info.style.clone(),
        _ => None,
    };
    let plain: Vec<_> = {
        let d = ada.view().unwrap();
        d.paragraphs()
            .into_iter()
            .filter(|p| style(d, *p).is_none() && !d.text(*p).unwrap().is_empty())
            .take(2)
            .collect()
    };
    let set = |from, to| Command::SetParagraphStyle {
        from: Point::Start { start: from },
        to: Point::End { end: to },
        style: "Heading1".into(),
    };
    ada.apply(&set(plain[0], plain[0])).unwrap();
    sync(&mut ada, &mut ben, false);
    ben.apply(&set(plain[0], plain[1])).unwrap();
    sync(&mut ada, &mut ben, true);
    assert!(ada.undo().unwrap());
    sync(&mut ada, &mut ben, false);
    let d = ben.view().unwrap();
    for p in plain {
        assert_eq!(style(d, p).as_deref(), Some("Heading1"));
    }
}
