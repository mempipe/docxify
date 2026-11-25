#[path = "../../docx-engine/tests/common/mod.rs"]
mod common;
use common::*;
use docx_engine::{Command, Format, Point, Track};
use docx_engine_yrs::Replica;
fn sync(a: &mut Replica, b: &mut Replica) {
    let ua = a.update_since(&b.state_vector()).unwrap();
    let ub = b.update_since(&a.state_vector()).unwrap();
    a.apply_update(&ub).unwrap();
    b.apply_update(&ua).unwrap();
    assert!(a.export().unwrap() == b.export().unwrap());
    assert_valid("strands", &a.export().unwrap());
}
#[test]
fn annotations_cross_source_strands_and_deleted_marks_without_moving_text() {
    let bytes = generated("simple-paragraphs");
    let mut a = Replica::seed(1, &bytes).unwrap();
    let original = a.export().unwrap();
    let mut b = Replica::join(2, &bytes, &a.save()).unwrap();
    let ps = a.view().unwrap().paragraphs();
    a.apply(&Command::AddComment {
        from: Point::Start { start: ps[0] },
        to: Point::End { end: ps[1] },
        text: "Both".into(),
        by: Track {
            author: "A".into(),
            date: "2026-11-18T00:00:00Z".into(),
        },
        initials: None,
        id: None,
    })
    .unwrap();
    b.apply(&Command::MergeParagraphs { paragraph: ps[0] })
        .unwrap();
    sync(&mut a, &mut b);
    let doc = a.view().unwrap();
    assert_eq!(
        doc.text(doc.paragraphs()[0]).unwrap(),
        "First paragraph.Second paragraph."
    );
    let xml = part(&a.export().unwrap(), "word/document.xml");
    assert_eq!(xml.matches("<w:commentRangeStart ").count(), 1);
    assert_eq!(xml.matches("<w:commentRangeEnd ").count(), 1);
    a.undo().unwrap();
    b.undo().unwrap();
    sync(&mut a, &mut b);
    assert!(a.export().unwrap() == original);
    a.apply(&Command::FormatRange {
        from: Point::Start { start: ps[0] },
        to: Point::End { end: ps[1] },
        format: Format {
            bold: Some(true),
            ..Default::default()
        },
    })
    .unwrap();
    b.apply(&Command::ReplaceRange {
        from: Point::Start { start: ps[0] },
        to: Point::End { end: ps[1] },
        text: "Replacement\nTail".into(),
        track: None,
    })
    .unwrap();
    sync(&mut a, &mut b);
    a.undo().unwrap();
    b.undo().unwrap();
    sync(&mut a, &mut b);
    assert!(a.export().unwrap() == original);
}
