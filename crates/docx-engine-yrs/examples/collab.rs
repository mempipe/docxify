//! Two replicas edit concurrently, converge and export a valid DOCX.
//! cargo run --release -p docx-engine-yrs --example collab <in.docx> [out.docx]
use docx_engine::{Command, NodeKind, Point, Support};
use docx_engine_yrs::Replica;
fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("collab <in.docx> [out.docx]");
    let bytes = std::fs::read(path).unwrap();
    let mut a = Replica::seed(1, &bytes).unwrap();
    let mut b = Replica::join(2, &bytes, &a.save()).unwrap();
    let text = |r: &mut Replica| {
        let d = r.view().unwrap();
        d.stories()
            .flat_map(|s| d.descendants(s.id))
            .find(|id| {
                matches!(d.node(*id).unwrap().kind, NodeKind::Text { .. })
                    && d.node(*id).unwrap().support == Support::Editable
            })
            .expect("editable text")
    };
    let an = text(&mut a);
    let bn = text(&mut b);
    a.apply(&Command::InsertText {
        at: Point::Text {
            node: an,
            offset: 0,
        },
        text: "Alice ".into(),
    })
    .unwrap();
    b.apply(&Command::InsertText {
        at: Point::Text {
            node: bn,
            offset: 0,
        },
        text: "Bob ".into(),
    })
    .unwrap();
    let ua = a.update_since(&b.state_vector()).unwrap();
    let ub = b.update_since(&a.state_vector()).unwrap();
    a.apply_update(&ub).unwrap();
    b.apply_update(&ua).unwrap();
    let out = a.export().unwrap();
    assert!(out == b.export().unwrap());
    assert!(docx_engine::validate_package(&out).valid);
    println!(
        "Converged: {} output bytes; {} shared-state bytes",
        out.len(),
        a.save().len()
    );
    if let Some(path) = args.next() {
        std::fs::write(path, out).unwrap();
    }
}
