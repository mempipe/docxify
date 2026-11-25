use docx_engine::{Command, Document, NodeKind, Point, Support, TextRange};
use docx_engine_yrs::Replica;
use std::path::PathBuf;
fn fixtures() -> Vec<PathBuf> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures");
    let mut paths = Vec::new();
    for dir in ["generated", "real-world"] {
        paths.extend(
            std::fs::read_dir(root.join(dir))
                .unwrap()
                .map(|e| e.unwrap().path())
                .filter(|p| p.extension().is_some_and(|e| e == "docx")),
        );
    }
    paths.sort();
    paths
}
#[test]
fn every_fixture_opens_preserves_bytes_and_can_edit_its_supported_regions() {
    let mut count = 0;
    for path in fixtures() {
        let bytes = std::fs::read(&path).unwrap();
        let base = Document::open(&bytes).unwrap();
        let mut a = Replica::seed(1, &bytes).unwrap();
        let before = a.export().unwrap();
        assert_eq!(before, base.export().unwrap(), "{}", path.display());
        let initial = a.save();
        let mut tested = std::collections::HashSet::new();
        for p in base.paragraphs() {
            if base.node(p).unwrap().support != Support::Editable {
                continue;
            }
            let parent = base.node(p).unwrap().parent.unwrap();
            if !tested.insert(parent) {
                continue;
            }
            let text = base.descendants(p).into_iter().find(|id| {
                let node = base.node(*id).unwrap();
                matches!(node.kind, NodeKind::Text { .. }) && node.support == Support::Editable
            });
            let Some(node) = text else {
                continue;
            };
            a.apply(&Command::ReplaceText {
                node,
                range: Some(TextRange { start: 0, end: 0 }),
                text: "🦀region".into(),
            })
            .unwrap_or_else(|e| panic!("{} {p:?}: {e}", path.display()));
            let cached = a.view().unwrap().export().unwrap();
            let out = a.export().unwrap();
            // ZIP entry metadata can differ after an incremental edit versus
            // a fresh build. Every package part must still have the same bytes.
            fn parts(bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
                use std::io::Read;
                let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
                (0..zip.len())
                    .map(|i| {
                        let mut entry = zip.by_index(i).unwrap();
                        let mut data = Vec::new();
                        entry.read_to_end(&mut data).unwrap();
                        (entry.name().unwrap().into_owned(), data)
                    })
                    .collect()
            }
            assert!(
                parts(&cached) == parts(&out),
                "{} cached/fresh parts {p:?}",
                path.display()
            );
            assert!(
                docx_engine::validate_package(&out).valid,
                "{}",
                path.display()
            );
            let reopened = Document::open(&out).unwrap();
            assert!(reopened
                .paragraphs()
                .iter()
                .any(|id| reopened.text(*id).unwrap().contains("🦀region")));
            let mut b = Replica::join(2, &bytes, &initial).unwrap();
            b.apply_update(&a.update_since(&b.state_vector()).unwrap())
                .unwrap();
            assert_eq!(out, b.export().unwrap());
            a.undo().unwrap();
            assert_eq!(a.export().unwrap(), before, "{} undo", path.display());
        }
        count += 1;
    }
    assert_eq!(count, 31);
}
#[test]
fn a_range_cannot_delete_across_table_cells_or_stories() {
    for name in ["table", "header-footer"] {
        let bytes = std::fs::read(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join(format!("../../fixtures/generated/{name}.docx")),
        )
        .unwrap();
        let mut a = Replica::seed(1, &bytes).unwrap();
        let doc = a.view().unwrap();
        let ps = doc.paragraphs();
        let pair = ps
            .windows(2)
            .find(|p| doc.node(p[0]).unwrap().parent != doc.node(p[1]).unwrap().parent)
            .unwrap();
        let command = Command::ReplaceRange {
            from: Point::Start { start: pair[0] },
            to: Point::End { end: pair[1] },
            text: "".into(),
            track: None,
        };
        let before = a.save();
        assert!(a.apply(&command).is_err());
        assert_eq!(a.save(), before);
    }
}
