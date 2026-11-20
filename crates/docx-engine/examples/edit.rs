//! Replace a text node's text and write the exported DOCX.
//!
//!     cargo run -p docx-engine --example edit -- <in.docx> <out.docx> [node-id] [text]
//!
//! Without a node ID, the first editable text node is edited. Prints the
//! edited node's ID.

use docx_engine::{editing::Editor, Command, NodeKind, Support};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [input, output, rest @ ..] = args.as_slice() else {
        eprintln!("usage: edit <in.docx> <out.docx> [node-id] [text]");
        std::process::exit(2);
    };
    let mut editor = Editor::open(&std::fs::read(input).unwrap()).unwrap_or_else(|e| {
        eprintln!("{input}: {e}");
        std::process::exit(1);
    });
    let doc = editor.view().unwrap();
    let node = match rest.first() {
        Some(id) => id.parse().unwrap(),
        None => {
            let roots: Vec<_> = doc.stories().map(|s| s.id).collect();
            roots
                .into_iter()
                .flat_map(|r| doc.descendants(r))
                .find(|&id| {
                    let n = doc.node(id).unwrap();
                    n.support == Support::Editable
                        && matches!(&n.kind, NodeKind::Text { text } if !text.is_empty())
                })
                .expect("no editable text")
        }
    };
    let text = rest
        .get(1)
        .cloned()
        .unwrap_or_else(|| "Edited by docx-engine".to_string());
    if let Err(e) = editor.apply(&Command::ReplaceText {
        node,
        range: None,
        text,
    }) {
        eprintln!("{e}");
        std::process::exit(1);
    }
    std::fs::write(output, editor.export().unwrap()).unwrap();
    println!("{node}");
}
