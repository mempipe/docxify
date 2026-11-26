#!/usr/bin/env bash
# Cross-check exports with an independent reader. For every fixture, edit the
# first editable text node with the engine, then open the original and the
# export with python-docx and compare paragraph text in the body, tables,
# headers and footers. Exactly one paragraph may differ: the edited one.
#
#   prototypes/docx-engine/scripts/check-with-python-docx.sh
#
# Needs uv; downloads python-docx 1.2.0 on first run.
set -euo pipefail
cd "$(dirname "$0")/.."
out="$(mktemp -d)"
trap 'rm -rf "$out"' EXIT
cargo build -q -p docx-engine --example edit
for f in fixtures/generated/*.docx fixtures/real-world/*.docx; do
  ./target/debug/examples/edit "$f" "$out/$(basename "$f")" >/dev/null
done
uv run -q --no-project --with python-docx==1.2.0 python - "$out" fixtures/generated/*.docx fixtures/real-world/*.docx <<'PY'
import os, sys
from docx import Document

def texts(path):
    doc = Document(path)
    out = []
    def blocks(container):
        for p in container.paragraphs:
            out.append(p.text)
        for t in getattr(container, "tables", []):
            for row in t.rows:
                for cell in row.cells:
                    blocks(cell)
    blocks(doc)
    for s in doc.sections:
        blocks(s.header)
        blocks(s.footer)
    return out

out_dir, *inputs = sys.argv[1:]
failed = 0
for path in inputs:
    before = texts(path)
    after = texts(os.path.join(out_dir, os.path.basename(path)))
    changed = [i for i, (a, b) in enumerate(zip(before, after)) if a != b]
    ok = len(before) == len(after) and len(changed) <= 1 and all("Edited by docx-engine" in after[i] for i in changed)
    failed += not ok
    print(f"{'ok' if ok else 'FAIL'} {path}: {len(before)} paragraphs, {len(changed)} changed")
sys.exit(1 if failed else 0)
PY
