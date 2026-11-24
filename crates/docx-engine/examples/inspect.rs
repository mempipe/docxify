//! Print a document's inspection JSON.
//!
//!     cargo run -p docx-engine --example inspect -- path/to/file.docx

fn main() {
    let path = std::env::args().nth(1).expect("usage: inspect <file.docx>");
    let bytes = std::fs::read(&path).expect("read input");
    match docx_engine::Document::open(&bytes) {
        Ok(doc) => println!("{}", serde_json::to_string_pretty(&doc.inspect()).unwrap()),
        Err(e) => {
            eprintln!("{path}: {e}");
            std::process::exit(1);
        }
    }
}
