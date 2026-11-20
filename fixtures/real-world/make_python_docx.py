"""Write python-docx.docx: a document from python-docx's Word-authored template.

    uv run --no-project --with python-docx==1.2.0 python make_python_docx.py <out.docx>
"""

import struct
import sys
import tempfile
import zlib

from docx import Document
from docx.shared import Pt


def png(path):
    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))

    raw = b"\x00\xff\x00\x00" * 1
    with open(path, "wb") as f:
        f.write(b"\x89PNG\r\n\x1a\n")
        f.write(chunk(b"IHDR", struct.pack(">IIBBBBB", 1, 1, 8, 2, 0, 0, 0)))
        f.write(chunk(b"IDAT", zlib.compress(raw)))
        f.write(chunk(b"IEND", b""))


doc = Document()
doc.core_properties.title = "python-docx fixture"
doc.add_heading("Quarterly report", 0)
doc.add_heading("Summary", level=1)
p = doc.add_paragraph("Revenue grew ")
p.add_run("twelve percent").bold = True
p.add_run(" while costs stayed ")
p.add_run("flat").italic = True
p.add_run(".")
doc.add_paragraph("First finding", style="List Bullet")
doc.add_paragraph("Second finding", style="List Bullet")
doc.add_paragraph("Step one", style="List Number")
doc.add_paragraph("Step two", style="List Number")
table = doc.add_table(rows=3, cols=3)
table.style = "Table Grid"
for r, row in enumerate(table.rows):
    for c, cell in enumerate(row.cells):
        cell.text = f"R{r + 1}C{c + 1}"
with tempfile.NamedTemporaryFile(suffix=".png") as image:
    png(image.name)
    doc.add_picture(image.name)
run = doc.add_paragraph().add_run("Large closing line.")
run.font.size = Pt(16)
section = doc.sections[0]
section.header.paragraphs[0].text = "Confidential draft"
section.footer.paragraphs[0].text = "Mempipe docx-engine fixture"
doc.save(sys.argv[1])
