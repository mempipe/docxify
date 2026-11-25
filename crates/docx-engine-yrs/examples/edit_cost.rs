//! Local editing cost, including command preparation and materialization.
//! Run with --release; record the machine load beside the output.
#[path = "../../docx-engine/tests/common/mod.rs"]
mod common;
use docx_engine::{Command, Point};
use docx_engine_yrs::Replica;
use std::time::Instant;
fn main() {
    if std::env::args().any(|arg| arg == "--delete-paragraphs") {
        let paragraph = "<w:p><w:r><w:t>The editor keeps a paragraph with its original formatting and text.</w:t></w:r></w:p>";
        for count in [100, 500, 1_000, 4_000] {
            let bytes = common::with_body(&paragraph.repeat(count));
            let mut times = [0; 3];
            for time in &mut times {
                let mut replica = Replica::seed(1, &bytes).unwrap();
                let paragraphs = replica.view().unwrap().paragraphs();
                let delete = Command::ReplaceRange {
                    from: Point::Start {
                        start: paragraphs[0],
                    },
                    to: Point::End {
                        end: paragraphs[count - 1],
                    },
                    text: String::new(),
                    track: None,
                };
                let now = Instant::now();
                replica.apply(&delete).unwrap();
                *time = now.elapsed().as_micros();
                assert_eq!(replica.view().unwrap().paragraphs().len(), 1);
                assert!(replica.undo().unwrap());
                assert_eq!(replica.view().unwrap().paragraphs().len(), count);
            }
            times.sort_unstable();
            println!("delete {count} paragraphs: median {} us", times[1]);
        }
        return;
    }
    if std::env::args().any(|arg| arg == "--positions") {
        let xml = "<w:p><w:r><w:t>Some paragraph text.</w:t></w:r></w:p>".repeat(10_000);
        let bytes = common::with_body(&xml);
        let mut replica = Replica::seed(1, &bytes).unwrap();
        let last = replica.view().unwrap().paragraphs()[9_999];
        let point = Point::End { end: last };
        let (key, token) = replica.sticky(&point).unwrap().unwrap();
        for label in ["capture", "resolve"] {
            let mut samples = [0; 31];
            for sample in &mut samples {
                let now = Instant::now();
                for _ in 0..20 {
                    if label == "capture" {
                        std::hint::black_box(replica.sticky(&point).unwrap().unwrap());
                    } else {
                        std::hint::black_box(replica.resolve(&key, &token).unwrap().unwrap());
                    }
                }
                *sample = now.elapsed().as_nanos() / 20;
            }
            samples.sort();
            println!("10,000 paragraphs, {label} final caret: {} ns", samples[15]);
        }
        return;
    }
    if std::env::args().any(|arg| arg == "--sustained") {
        for count in [100, 10_000] {
            let xml = "<w:p><w:r><w:t>Some paragraph text.</w:t></w:r></w:p>".repeat(count);
            let mut replica = Replica::seed(1, &common::with_body(&xml)).unwrap();
            let p = replica.view().unwrap().paragraphs()[count / 2];
            let key = Command::InsertText {
                at: Point::End { end: p },
                text: "x".into(),
            };
            let mut typed = 0;
            for start in [0, 100, 200, 400, 800] {
                while typed < start {
                    replica.apply(&key).unwrap();
                    typed += 1;
                }
                let mut times = [0; 20];
                for time in &mut times {
                    let now = Instant::now();
                    replica.apply(&key).unwrap();
                    *time = now.elapsed().as_micros();
                    typed += 1;
                }
                times.sort();
                println!(
                    "{count} paragraphs, after {start} typed characters: median key {} us",
                    times[10]
                );
            }
        }
        return;
    }
    if std::env::args().any(|arg| arg == "--properties") {
        return property_edits();
    }
    // A long, isolated sample window for a system profiler.
    let profile = std::env::args().any(|arg| arg == "--profile-keys");
    let counts: &[usize] = if profile {
        &[10_000]
    } else {
        &[10, 1000, 10000]
    };
    for &count in counts {
        let xml = "<w:p><w:r><w:t>Some paragraph text.</w:t></w:r></w:p>".repeat(count);
        let bytes = common::with_body(&xml);
        let mut d = Replica::seed(1, &bytes).unwrap();
        let p = d.view().unwrap().paragraphs()[count / 2];
        let mut times = Vec::new();
        for _ in 0..if profile { 500 } else { 11 } {
            let start = Instant::now();
            d.apply(&Command::InsertText {
                at: Point::End { end: p },
                text: "x".into(),
            })
            .unwrap();
            times.push(start.elapsed().as_micros());
        }
        times.sort();
        println!(
            "{count} paragraphs: median key {} us",
            times[times.len() / 2]
        );
        if profile {
            return;
        }
        let start = Instant::now();
        d.apply(&Command::SplitParagraph {
            at: Point::End { end: p },
        })
        .unwrap();
        println!(
            "{count} paragraphs: Enter {} us",
            start.elapsed().as_micros()
        );
        d.apply(&Command::AddComment {
            from: Point::Start { start: p },
            to: Point::End { end: p },
            text: "Review".into(),
            by: docx_engine::Track {
                author: "Ada".into(),
                date: "2026-11-20T00:00:00Z".into(),
            },
            initials: None,
            id: None,
        })
        .unwrap();
        let mut times = Vec::new();
        for _ in 0..11 {
            let view = d.view().unwrap();
            let leaf = view
                .descendants(p)
                .into_iter()
                .find(|id| view.editable_text(*id).is_ok_and(|s| s.len() > 1))
                .unwrap();
            let start = Instant::now();
            d.apply(&Command::InsertText {
                at: common::at(leaf, 1),
                text: "x".into(),
            })
            .unwrap();
            times.push(start.elapsed().as_micros());
        }
        times.sort();
        println!(
            "{count} paragraphs: median key inside comment {} us",
            times[5]
        );
    }
    let bytes = common::with_body("<w:p><w:r><w:t>End</w:t></w:r></w:p>");
    let mut d = Replica::seed(1, &bytes).unwrap();
    let p = d.view().unwrap().paragraphs()[0];
    d.apply(&Command::InsertText {
        at: Point::Start { start: p },
        text: "pasted\n".repeat(1000),
    })
    .unwrap();
    let p = d.view().unwrap().paragraphs()[500];
    let mut times = Vec::new();
    for _ in 0..11 {
        let start = Instant::now();
        d.apply(&Command::InsertText {
            at: Point::Start { start: p },
            text: "x".into(),
        })
        .unwrap();
        times.push(start.elapsed().as_micros());
    }
    times.sort();
    println!(
        "1000 pasted paragraphs: median key {} us",
        times[times.len() / 2]
    );
    undone_table_commands();
}

/// Undo keeps an undone table command in the store, and every build folds
/// each stored table command into its table. This measures what that costs a
/// later command.
fn undone_table_commands() {
    use docx_engine::NodeKind;
    let cell = "<w:tc><w:p><w:r><w:t>cell</w:t></w:r></w:p></w:tc>";
    let bytes = common::with_body(&format!(
        "<w:p><w:r><w:t>Body text.</w:t></w:r></w:p><w:tbl><w:tblGrid>{}</w:tblGrid>{}</w:tbl><w:p/>",
        r#"<w:gridCol w:w="3000"/>"#.repeat(2),
        format!("<w:tr>{cell}{cell}</w:tr>").repeat(2)
    ));
    let median = |mut times: Vec<u128>| {
        times.sort();
        times[times.len() / 2]
    };
    for undone in [0, 100, 400, 1600] {
        let mut d = Replica::seed(1, &bytes).unwrap();
        let view = d.view().unwrap();
        let table = (view.stories().flat_map(|s| view.descendants(s.id)))
            .find(|id| matches!(view.node(*id).unwrap().kind, NodeKind::Table(_)))
            .unwrap();
        let row = view.node(table).unwrap().children[0];
        let body = view.paragraphs()[0];
        let insert = Command::InsertRow {
            row,
            below: true,
            like: None,
        };
        for _ in 0..undone {
            d.apply(&insert).unwrap();
            assert!(d.undo().unwrap());
        }
        let (mut rows, mut keys) = (Vec::new(), Vec::new());
        for _ in 0..11 {
            let start = Instant::now();
            d.apply(&insert).unwrap();
            rows.push(start.elapsed().as_micros());
            assert!(d.undo().unwrap());
        }
        // Keys in a row, so only the first pays for the view the undo dropped.
        for _ in 0..11 {
            let start = Instant::now();
            d.apply(&Command::InsertText {
                at: Point::End { end: body },
                text: "x".into(),
            })
            .unwrap();
            keys.push(start.elapsed().as_micros());
        }
        println!(
            "{undone} undone row insertions: median row insertion {} us, median key {} us",
            median(rows),
            median(keys)
        );
    }
}

/// Edits that read and rewrite run, cell and row properties.
fn property_edits() {
    use docx_engine::{Format, NodeKind};
    let bold = || Format {
        bold: Some(true),
        ..Format::default()
    };
    let median = |mut times: Vec<u128>| {
        times.sort();
        times[times.len() / 2]
    };
    let key = |d: &mut Replica, p| {
        let mut times = Vec::new();
        for _ in 0..21 {
            let start = Instant::now();
            d.apply(&Command::InsertText {
                at: Point::End { end: p },
                text: "x".into(),
            })
            .unwrap();
            times.push(start.elapsed().as_micros());
        }
        median(times)
    };
    let properties = r#"<w:rPr><w:i/><w:sz w:val="24"/></w:rPr>"#;
    let run = format!("<w:r>{properties}<w:t>Some paragraph text.</w:t></w:r>");
    let xml = format!("<w:p>{run}</w:p>").repeat(2_000);
    let mut d = Replica::seed(1, &common::with_body(&xml)).unwrap();
    let paragraphs = d.view().unwrap().paragraphs();
    let start = Instant::now();
    d.apply(&Command::FormatRange {
        from: Point::Start {
            start: paragraphs[0],
        },
        to: Point::End {
            end: paragraphs[1_999],
        },
        format: bold(),
    })
    .unwrap();
    println!("bold 2,000 paragraphs: {} us", start.elapsed().as_micros());
    let p = d.view().unwrap().paragraphs()[1_000];
    println!("key after the bold: median {} us", key(&mut d, p));

    // One paragraph of a hundred words, with fifty bold stretches made one
    // command at a time.
    let words = "word ".repeat(100);
    let xml =
        format!(r#"<w:p><w:r>{properties}<w:t xml:space="preserve">{words}</w:t></w:r></w:p>"#);
    let mut d = Replica::seed(1, &common::with_body(&xml)).unwrap();
    let mut times = Vec::new();
    for word in 0..50 {
        let view = d.view().unwrap();
        let p = view.paragraphs()[0];
        let leaf = (view.descendants(p).into_iter())
            .rfind(|id| view.editable_text(*id).is_ok_and(|s| s.len() > 5))
            .unwrap();
        let from = if word == 0 { 0 } else { 5 };
        let start = Instant::now();
        d.apply(&Command::FormatRange {
            from: common::at(leaf, from),
            to: common::at(leaf, from + 4),
            format: bold(),
        })
        .unwrap();
        times.push(start.elapsed().as_micros());
    }
    println!(
        "bold one word of a paragraph, 50 times: median {} us",
        median(times)
    );
    let p = d.view().unwrap().paragraphs()[0];
    println!(
        "key in a paragraph of 100 runs: median {} us",
        key(&mut d, p)
    );

    let cell = r#"<w:tc><w:tcPr><w:tcW w:w="2000" w:type="dxa"/></w:tcPr><w:p><w:r><w:t>cell</w:t></w:r></w:p></w:tc>"#;
    let bytes = common::with_body(&format!(
        "<w:tbl><w:tblGrid>{}</w:tblGrid>{}</w:tbl><w:p/>",
        r#"<w:gridCol w:w="2000"/>"#.repeat(4),
        format!("<w:tr>{}</w:tr>", cell.repeat(4)).repeat(300)
    ));
    let mut d = Replica::seed(1, &bytes).unwrap();
    let view = d.view().unwrap();
    let table = (view.stories().flat_map(|s| view.descendants(s.id)))
        .find(|id| matches!(view.node(*id).unwrap().kind, NodeKind::Table(_)))
        .unwrap();
    let row = view.node(table).unwrap().children[0];
    let start = Instant::now();
    d.apply(&Command::InsertColumn {
        table,
        index: 1,
        like: 1,
    })
    .unwrap();
    println!("column into 300 rows: {} us", start.elapsed().as_micros());
    let start = Instant::now();
    d.apply(&Command::InsertRow {
        row,
        below: true,
        like: None,
    })
    .unwrap();
    println!("row into 300 rows: {} us", start.elapsed().as_micros());
}
