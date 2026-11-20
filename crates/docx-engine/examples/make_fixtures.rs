//! Writes the generated fixture corpus to `fixtures/generated/`.
//!
//!     cargo run -p docx-engine --example make_fixtures
//!
//! Each fixture is hand-written OOXML covering one row of the spike's fixture
//! table. ZIP timestamps are fixed, so the output is reproducible.

use std::io::{Cursor, Write};
use std::path::Path;

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, ZipWriter};

const NS: &str = concat!(
    r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" "#,
    r#"xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" "#,
    r#"xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" "#,
    r#"xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" "#,
    r#"xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture" "#,
    r#"xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml" "#,
    r#"xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" "#,
    r#"xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math" "#,
    r#"xmlns:x="urn:example:unknown" mc:Ignorable="w14 x""#
);
const DECL: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#;
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const SECT: &str = r#"<w:sectPr><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="720" w:footer="720" w:gutter="0"/></w:sectPr>"#;

/// A 1×1 PNG.
const PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4,
    0x89, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0xF8, 0xCF, 0xC0, 0xF0,
    0x1F, 0x00, 0x05, 0x00, 0x01, 0xFF, 0x89, 0x99, 0x3D, 0x1D, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45,
    0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
];

#[derive(Default)]
struct Fixture {
    body: String,
    rels: Vec<(String, String, String, bool)>,
    overrides: Vec<(String, String)>,
    parts: Vec<(String, Vec<u8>)>,
    styles: Option<String>,
    numbering: Option<String>,
}

impl Fixture {
    fn new(body: impl Into<String>) -> Self {
        Fixture {
            body: body.into(),
            ..Fixture::default()
        }
    }

    fn rel(mut self, id: &str, kind: &str, target: &str) -> Self {
        self.rels
            .push((id.into(), format!("{REL}/{kind}"), target.into(), false));
        self
    }

    fn external(mut self, id: &str, kind: &str, target: &str) -> Self {
        self.rels
            .push((id.into(), format!("{REL}/{kind}"), target.into(), true));
        self
    }

    fn part(mut self, name: &str, content_type: Option<&str>, data: impl Into<Vec<u8>>) -> Self {
        if let Some(ct) = content_type {
            self.overrides.push((name.into(), ct.into()));
        }
        self.parts.push((name.into(), data.into()));
        self
    }

    fn with_numbering(self) -> Self {
        self.with_numbering_xml(numbering())
    }

    fn with_numbering_xml(mut self, numbering: String) -> Self {
        self.numbering = Some(numbering);
        self
    }

    fn with_styles(mut self, styles: String) -> Self {
        self.styles = Some(styles);
        self
    }

    fn build(self) -> Vec<u8> {
        let mut rels = vec![(
            "rIdStyles".to_string(),
            format!("{REL}/styles"),
            "styles.xml".to_string(),
            false,
        )];
        let mut overrides = vec![
            (
                "/word/document.xml".to_string(),
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"
                    .to_string(),
            ),
            (
                "/word/styles.xml".to_string(),
                "application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"
                    .to_string(),
            ),
            (
                "/docProps/core.xml".to_string(),
                "application/vnd.openxmlformats-package.core-properties+xml".to_string(),
            ),
        ];
        let mut parts: Vec<(String, Vec<u8>)> = vec![(
            "word/styles.xml".into(),
            self.styles.unwrap_or_else(styles).into_bytes(),
        )];
        if let Some(numbering) = self.numbering {
            rels.push((
                "rIdNumbering".into(),
                format!("{REL}/numbering"),
                "numbering.xml".into(),
                false,
            ));
            overrides.push((
                "/word/numbering.xml".into(),
                "application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml"
                    .into(),
            ));
            parts.push(("word/numbering.xml".into(), numbering.into_bytes()));
        }
        rels.extend(self.rels);
        overrides.extend(
            self.overrides
                .into_iter()
                .map(|(n, ct)| (format!("/{n}"), ct)),
        );
        parts.extend(self.parts);

        let mut ct = format!(
            r#"{DECL}<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Default Extension="png" ContentType="image/png"/>"#
        );
        for (name, t) in &overrides {
            ct.push_str(&format!(
                r#"<Override PartName="{name}" ContentType="{t}"/>"#
            ));
        }
        ct.push_str("</Types>");

        let package_rels = format!(
            r#"{DECL}<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{REL}/officeDocument" Target="word/document.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties" Target="docProps/core.xml"/></Relationships>"#
        );
        let mut doc_rels = format!(
            r#"{DECL}<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#
        );
        for (id, kind, target, external) in &rels {
            let mode = if *external {
                r#" TargetMode="External""#
            } else {
                ""
            };
            doc_rels.push_str(&format!(
                r#"<Relationship Id="{id}" Type="{kind}" Target="{target}"{mode}/>"#
            ));
        }
        doc_rels.push_str("</Relationships>");
        let core = format!(
            r#"{DECL}<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>docx-engine fixture</dc:title><dc:creator>docx-engine</dc:creator></cp:coreProperties>"#
        );
        let document = format!(
            r#"{DECL}<w:document {NS}><w:body>{}</w:body></w:document>"#,
            self.body
        );

        let mut entries: Vec<(String, Vec<u8>)> = vec![
            ("[Content_Types].xml".into(), ct.into_bytes()),
            ("_rels/.rels".into(), package_rels.into_bytes()),
            ("docProps/core.xml".into(), core.into_bytes()),
            ("word/document.xml".into(), document.into_bytes()),
            ("word/_rels/document.xml.rels".into(), doc_rels.into_bytes()),
        ];
        entries.extend(parts);
        zip(&entries)
    }
}

fn zip(entries: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .last_modified_time(DateTime::default());
    for (name, data) in entries {
        writer.start_file(name.as_str(), options).unwrap();
        writer.write_all(data).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

fn styles() -> String {
    format!(
        r#"{DECL}<w:styles {NS}><w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii="Calibri" w:hAnsi="Calibri"/><w:sz w:val="22"/></w:rPr></w:rPrDefault><w:pPrDefault><w:pPr><w:spacing w:after="160"/></w:pPr></w:pPrDefault></w:docDefaults><w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style><w:style w:type="paragraph" w:styleId="Title"><w:name w:val="Title"/><w:basedOn w:val="Normal"/><w:rPr><w:sz w:val="56"/></w:rPr></w:style><w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/><w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:pPr><w:keepNext/><w:outlineLvl w:val="0"/></w:pPr><w:rPr><w:b/><w:sz w:val="32"/><w:color w:val="2F5496"/></w:rPr></w:style><w:style w:type="paragraph" w:styleId="Heading2"><w:name w:val="heading 2"/><w:basedOn w:val="Heading1"/><w:pPr><w:outlineLvl w:val="1"/></w:pPr><w:rPr><w:sz w:val="26"/></w:rPr></w:style><w:style w:type="paragraph" w:styleId="ListBullet"><w:name w:val="List Bullet"/><w:basedOn w:val="Normal"/><w:pPr><w:numPr><w:numId w:val="1"/></w:numPr></w:pPr></w:style><w:style w:type="character" w:styleId="Emphasis"><w:name w:val="Emphasis"/><w:rPr><w:i/></w:rPr></w:style><w:style w:type="table" w:styleId="TableGrid"><w:name w:val="Table Grid"/></w:style></w:styles>"#
    )
}

fn numbering() -> String {
    format!(
        r#"{DECL}<w:numbering {NS}><w:abstractNum w:abstractNumId="10"><w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="bullet"/><w:lvlText w:val="•"/><w:pPr><w:ind w:left="720" w:hanging="360"/></w:pPr></w:lvl><w:lvl w:ilvl="1"><w:start w:val="1"/><w:numFmt w:val="bullet"/><w:lvlText w:val="o"/><w:pPr><w:ind w:left="1440" w:hanging="360"/></w:pPr></w:lvl></w:abstractNum><w:abstractNum w:abstractNumId="20"><w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="decimal"/><w:lvlText w:val="%1."/><w:pPr><w:ind w:left="720" w:hanging="360"/></w:pPr></w:lvl><w:lvl w:ilvl="1"><w:start w:val="1"/><w:numFmt w:val="lowerLetter"/><w:lvlText w:val="%2)"/><w:pPr><w:ind w:left="1440" w:hanging="360"/></w:pPr></w:lvl></w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="10"/></w:num><w:num w:numId="2"><w:abstractNumId w:val="20"/></w:num></w:numbering>"#
    )
}

fn p(text: &str) -> String {
    format!(r#"<w:p><w:r><w:t>{text}</w:t></w:r></w:p>"#)
}

fn styled(style: &str, text: &str) -> String {
    format!(r#"<w:p><w:pPr><w:pStyle w:val="{style}"/></w:pPr><w:r><w:t>{text}</w:t></w:r></w:p>"#)
}

fn list(num: u32, level: u32, text: &str) -> String {
    format!(
        r#"<w:p><w:pPr><w:numPr><w:ilvl w:val="{level}"/><w:numId w:val="{num}"/></w:numPr></w:pPr><w:r><w:t>{text}</w:t></w:r></w:p>"#
    )
}

fn image_run(rel: &str) -> String {
    format!(
        r#"<w:r><w:drawing><wp:inline distT="0" distB="0" distL="0" distR="0"><wp:extent cx="914400" cy="914400"/><wp:docPr id="1" name="Picture 1" descr="A one-pixel image"/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic><pic:nvPicPr><pic:cNvPr id="0" name="image1.png"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed="{rel}"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill><pic:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="914400" cy="914400"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></pic:spPr></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r>"#
    )
}

fn table() -> String {
    let cell = |text: &str, extra: &str| {
        format!(
            r#"<w:tc><w:tcPr><w:tcW w:w="3000" w:type="dxa"/>{extra}</w:tcPr><w:p><w:r><w:t>{text}</w:t></w:r></w:p></w:tc>"#
        )
    };
    let nested = r#"<w:tbl><w:tblPr><w:tblStyle w:val="TableGrid"/></w:tblPr><w:tblGrid><w:gridCol w:w="1400"/><w:gridCol w:w="1400"/></w:tblGrid><w:tr><w:tc><w:p><w:r><w:t>Inner A</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>Inner B</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#;
    format!(
        r#"<w:tbl><w:tblPr><w:tblStyle w:val="TableGrid"/><w:tblW w:w="0" w:type="auto"/></w:tblPr><w:tblGrid><w:gridCol w:w="3000"/><w:gridCol w:w="3000"/><w:gridCol w:w="3000"/></w:tblGrid><w:tr><w:trPr><w:tblHeader/></w:trPr>{}{}{}</w:tr><w:tr>{}{}</w:tr><w:tr>{}<w:tc><w:tcPr><w:tcW w:w="3000" w:type="dxa"/></w:tcPr>{nested}<w:p/></w:tc>{}</w:tr></w:tbl>"#,
        cell("Name", ""),
        cell("Role", ""),
        cell("Notes", ""),
        cell("Ada", r#"<w:vMerge w:val="restart"/>"#),
        cell("Spans two columns", r#"<w:gridSpan w:val="2"/>"#),
        cell("", "<w:vMerge/>"),
        cell("Last cell", ""),
    )
}

fn header_footer(f: Fixture) -> Fixture {
    let hdr = format!(
        r#"{DECL}<w:hdr {NS}><w:p><w:r><w:t xml:space="preserve">Header text, page </w:t></w:r><w:fldSimple w:instr=" PAGE "><w:r><w:t>1</w:t></w:r></w:fldSimple></w:p></w:hdr>"#
    );
    let ftr = format!(
        r#"{DECL}<w:ftr {NS}><w:p><w:r><w:t xml:space="preserve">Footer page </w:t></w:r><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve"> PAGE </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>1</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r></w:p></w:ftr>"#
    );
    f.rel("rIdHeader1", "header", "header1.xml")
        .rel("rIdFooter1", "footer", "footer1.xml")
        .part(
            "word/header1.xml",
            Some("application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml"),
            hdr,
        )
        .part(
            "word/footer1.xml",
            Some("application/vnd.openxmlformats-officedocument.wordprocessingml.footer+xml"),
            ftr,
        )
}

const SECT_HF: &str = r#"<w:sectPr><w:headerReference w:type="default" r:id="rIdHeader1"/><w:footerReference w:type="default" r:id="rIdFooter1"/><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="720" w:footer="720" w:gutter="0"/></w:sectPr>"#;

fn comments_part(f: Fixture) -> Fixture {
    let comments = format!(
        r#"{DECL}<w:comments {NS}><w:comment w:id="0" w:author="Reviewer" w:date="2026-11-14T00:00:00Z" w:initials="R"><w:p><w:r><w:t>Check this wording.</w:t></w:r></w:p></w:comment></w:comments>"#
    );
    f.rel("rIdComments", "comments", "comments.xml").part(
        "word/comments.xml",
        Some("application/vnd.openxmlformats-officedocument.wordprocessingml.comments+xml"),
        comments,
    )
}

/// A comment with a reply threaded under it, as Word 2013 and later write
/// one, and a comment from a producer that writes no paragraph IDs.
fn comments() -> Fixture {
    let body = concat!(
        r#"<w:p><w:r><w:t xml:space="preserve">A sentence with </w:t></w:r><w:commentRangeStart w:id="0"/><w:commentRangeStart w:id="1"/><w:r><w:t>a discussed phrase</w:t></w:r><w:commentRangeEnd w:id="0"/><w:r><w:commentReference w:id="0"/></w:r><w:commentRangeEnd w:id="1"/><w:r><w:commentReference w:id="1"/></w:r><w:r><w:t xml:space="preserve"> in it.</w:t></w:r></w:p>"#,
        r#"<w:p><w:commentRangeStart w:id="2"/><w:r><w:t>A paragraph whose comment has no paragraph ID.</w:t></w:r><w:commentRangeEnd w:id="2"/><w:r><w:commentReference w:id="2"/></w:r></w:p>"#,
        r#"<w:p><w:r><w:t>A plain paragraph.</w:t></w:r></w:p>"#,
    );
    let comments = format!(
        concat!(
            r#"{}<w:comments {}>"#,
            r#"<w:comment w:id="0" w:author="Reviewer" w:date="2026-11-14T00:00:00Z" w:initials="R"><w:p w14:paraId="10000001" w14:textId="77777777"><w:r><w:t>Is this the right phrase?</w:t></w:r></w:p></w:comment>"#,
            r#"<w:comment w:id="1" w:author="Author" w:date="2026-11-14T01:00:00Z" w:initials="A"><w:p w14:paraId="10000002" w14:textId="77777777"><w:r><w:t>Yes, it is.</w:t></w:r></w:p></w:comment>"#,
            r#"<w:comment w:id="2" w:author="Reviewer" w:date="2026-11-14T02:00:00Z" w:initials="R"><w:p><w:r><w:t>No paragraph ID here.</w:t></w:r></w:p></w:comment>"#,
            r#"</w:comments>"#
        ),
        DECL, NS
    );
    let extended = format!(
        concat!(
            r#"{}<w15:commentsEx xmlns:w15="http://schemas.microsoft.com/office/word/2012/wordml">"#,
            r#"<w15:commentEx w15:paraId="10000001" w15:done="0"/>"#,
            r#"<w15:commentEx w15:paraId="10000002" w15:paraIdParent="10000001" w15:done="0"/>"#,
            r#"</w15:commentsEx>"#
        ),
        DECL
    );
    let mut f = Fixture::new(format!("{body}{SECT}"))
        .rel("rIdComments", "comments", "comments.xml")
        .part(
            "word/comments.xml",
            Some("application/vnd.openxmlformats-officedocument.wordprocessingml.comments+xml"),
            comments,
        )
        .part(
            "word/commentsExtended.xml",
            Some("application/vnd.openxmlformats-officedocument.wordprocessingml.commentsExtended+xml"),
            extended,
        );
    f.rels.push((
        "rIdCommentsExtended".into(),
        "http://schemas.microsoft.com/office/2011/relationships/commentsExtended".into(),
        "commentsExtended.xml".into(),
        false,
    ));
    f
}

fn tracked() -> String {
    concat!(
        r#"<w:p><w:r><w:t xml:space="preserve">Kept text, </w:t></w:r>"#,
        r#"<w:ins w:id="1" w:author="Alice" w:date="2026-11-01T10:00:00Z"><w:r><w:t xml:space="preserve">inserted text, </w:t></w:r></w:ins>"#,
        r#"<w:del w:id="2" w:author="Bob" w:date="2026-11-02T11:00:00Z"><w:r><w:delText xml:space="preserve">deleted text, </w:delText></w:r></w:del>"#,
        r#"<w:r><w:rPr><w:b/><w:rPrChange w:id="3" w:author="Alice" w:date="2026-11-01T10:05:00Z"><w:rPr/></w:rPrChange></w:rPr><w:t>newly bold</w:t></w:r></w:p>"#,
        r#"<w:p><w:pPr><w:rPr><w:ins w:id="4" w:author="Alice" w:date="2026-11-01T10:06:00Z"/></w:rPr></w:pPr><w:ins w:id="5" w:author="Alice" w:date="2026-11-01T10:06:00Z"><w:r><w:t>A whole inserted paragraph.</w:t></w:r></w:ins></w:p>"#,
        r#"<w:p><w:moveFromRangeStart w:id="6" w:author="Bob" w:date="2026-11-02T11:01:00Z" w:name="move1"/><w:moveFrom w:id="7" w:author="Bob" w:date="2026-11-02T11:01:00Z"><w:r><w:t>Moved sentence.</w:t></w:r></w:moveFrom><w:moveFromRangeEnd w:id="6"/></w:p>"#,
        r#"<w:p><w:r><w:t xml:space="preserve">Destination: </w:t></w:r><w:moveToRangeStart w:id="8" w:author="Bob" w:date="2026-11-02T11:01:00Z" w:name="move1"/><w:moveTo w:id="9" w:author="Bob" w:date="2026-11-02T11:01:00Z"><w:r><w:t>Moved sentence.</w:t></w:r></w:moveTo><w:moveToRangeEnd w:id="8"/></w:p>"#,
        r#"<w:p><w:r><w:t>Untouched paragraph after the revisions.</w:t></w:r></w:p>"#
    )
    .to_string()
}

/// A form: a control of each type a fill takes, inline and block, with and
/// without a placeholder showing, and controls a fill refuses.
fn content_controls() -> String {
    let placeholder = |text: &str| {
        format!(r#"<w:r><w:rPr><w:rStyle w:val="PlaceholderText"/></w:rPr><w:t>{text}</w:t></w:r>"#)
    };
    let sdt = |props: &str, content: &str| {
        format!("<w:sdt><w:sdtPr>{props}</w:sdtPr><w:sdtContent>{content}</w:sdtContent></w:sdt>")
    };
    let run = |text: &str| format!(r#"<w:r><w:t xml:space="preserve">{text}</w:t></w:r>"#);
    let box_run = r#"<w:r><w:rPr><w:rFonts w:ascii="MS Gothic" w:eastAsia="MS Gothic" w:hAnsi="MS Gothic" w:hint="eastAsia"/></w:rPr><w:t>☐</w:t></w:r>"#;
    let checkbox = r#"<w14:checkbox><w14:checked w14:val="0"/><w14:checkedState w14:val="2612" w14:font="MS Gothic"/><w14:uncheckedState w14:val="2610" w14:font="MS Gothic"/></w14:checkbox>"#;
    let sizes = r#"<w:dropDownList><w:listItem w:displayText="Choose an item." w:value=""/><w:listItem w:displayText="Small" w:value="S"/><w:listItem w:displayText="Medium" w:value="M"/><w:listItem w:displayText="Large" w:value="L"/></w:dropDownList>"#;
    let colours = r#"<w:comboBox w:lastValue="Red"><w:listItem w:displayText="Red" w:value="Red"/><w:listItem w:displayText="Sea green" w:value="Green"/></w:comboBox>"#;
    let date = r#"<w:date><w:dateFormat w:val="dddd, MMMM d, yyyy"/><w:lid w:val="en-US"/><w:storeMappedDataAs w:val="dateTime"/><w:calendar w:val="gregorian"/></w:date>"#;
    [
        p("A form with content controls."),
        format!(
            "<w:p>{}{}</w:p>",
            run("Name: "),
            sdt(r#"<w:rPr><w:b/></w:rPr><w:alias w:val="Name"/><w:tag w:val="name"/><w:id w:val="101"/><w:showingPlcHdr/><w:text/>"#, &placeholder("Click to enter a name."))
        ),
        format!(
            "<w:p>{}{}</w:p>",
            sdt(&format!(r#"<w:alias w:val="Agree"/><w:tag w:val="agree"/><w:id w:val="102"/>{checkbox}"#), box_run),
            run(" I agree to the terms.")
        ),
        format!(
            "<w:p>{}{}</w:p>",
            run("Size: "),
            sdt(&format!(r#"<w:alias w:val="Size"/><w:tag w:val="size"/><w:id w:val="103"/><w:showingPlcHdr/>{sizes}"#), &placeholder("Choose an item."))
        ),
        format!(
            "<w:p>{}{}</w:p>",
            run("Colour: "),
            sdt(&format!(r#"<w:alias w:val="Colour"/><w:tag w:val="colour"/><w:id w:val="104"/>{colours}"#), &run("Red"))
        ),
        sdt(
            &format!(r#"<w:alias w:val="Start date"/><w:tag w:val="start"/><w:id w:val="105"/><w:showingPlcHdr/>{date}"#),
            &format!(r#"<w:p><w:pPr><w:jc w:val="center"/></w:pPr>{}</w:p>"#, placeholder("Click to enter a date.")),
        ),
        sdt(
            r#"<w:alias w:val="Address"/><w:tag w:val="address"/><w:id w:val="106"/><w:text w:multiLine="1"/>"#,
            r#"<w:p><w:r><w:rPr><w:i/></w:rPr><w:t>1 Old Street</w:t></w:r></w:p>"#,
        ),
        format!("<w:p>{}{}</w:p>", run("Notes: "), sdt(r#"<w:alias w:val="Notes"/><w:id w:val="107"/>"#, &run("None yet"))),
        format!(
            "<w:p>{}{}</w:p>",
            run("Reference: "),
            sdt(r#"<w:alias w:val="Reference"/><w:id w:val="108"/><w:lock w:val="contentLocked"/><w:text/>"#, &run("REF-0001"))
        ),
        format!(
            "<w:p>{}{}</w:p>",
            run("Company: "),
            sdt(
                r#"<w:alias w:val="Company"/><w:id w:val="109"/><w:dataBinding w:xpath="/root/company" w:storeItemID="{11111111-1111-1111-1111-111111111111}"/><w:text/>"#,
                &run("Bound company")
            )
        ),
        sdt(
            r#"<w:alias w:val="Signature block"/><w:id w:val="110"/>"#,
            &format!("<w:p>{}{}</w:p>", run("Signed: "), sdt(r#"<w:alias w:val="Signature"/><w:id w:val="111"/><w:text/>"#, &run("unsigned"))),
        ),
        format!("<w:p>{}{}</w:p>", run("Source: "), sdt(r#"<w:alias w:val="Source"/><w:id w:val="112"/><w:citation/>"#, &run("(Lovelace, 1843)"))),
    ]
    .concat()
}

fn unknown() -> String {
    concat!(
        r#"<w:p x:paraFlag="keep"><w:r><w:t>Before the unknown block.</w:t></w:r></w:p>"#,
        r#"<x:customBlock x:attr="1"><x:inner>keep me</x:inner></x:customBlock>"#,
        "<!-- a comment the engine must keep -->",
        r#"<w:p><w:r x:runFlag="keep"><w:t xml:space="preserve">Editable run, </w:t></w:r><x:inlineThing x:value="42"/><w:r><x:runThing/><w:t>after inline unknowns.</w:t></w:r></w:p>"#,
        r#"<w:sdt><w:sdtPr><w:alias w:val="Client name"/><w:tag w:val="client"/><w:dataBinding w:xpath="/root/client" w:storeItemID="{00000000-0000-0000-0000-000000000000}"/></w:sdtPr><w:sdtContent><w:p><w:r><w:t>Bound client name</w:t></w:r></w:p></w:sdtContent></w:sdt>"#,
        r#"<w:p><w:customXml w:element="party"><w:r><w:t>Custom XML wrapped run</w:t></w:r></w:customXml><w:r><w:t xml:space="preserve"> then plain text.</w:t></w:r></w:p>"#,
        r#"<w:p><w:r><w:t xml:space="preserve">Area: </w:t></w:r><m:oMath><m:r><m:t>πr²</m:t></m:r></m:oMath></w:p>"#,
        r#"<w:p><w:r><mc:AlternateContent><mc:Choice Requires="w14"><w14:someNewThing/></mc:Choice><mc:Fallback><w:t>fallback</w:t></mc:Fallback></mc:AlternateContent></w:r><w:r><w:t>Text next to alternate content.</w:t></w:r></w:p>"#,
        r#"<w:p><w:bookmarkStart w:id="0" w:name="intro"/><w:commentRangeStart w:id="0"/><w:r><w:t>Bookmarked and commented text.</w:t></w:r><w:commentRangeEnd w:id="0"/><w:r><w:commentReference w:id="0"/></w:r><w:bookmarkEnd w:id="0"/></w:p>"#,
        r#"<w:p><w:smartTag w:uri="urn:example" w:element="place"><w:r><w:t>Smart tagged</w:t></w:r></w:smartTag><w:proofErr w:type="spellStart"/><w:r><w:t xml:space="preserve"> speling</w:t></w:r><w:proofErr w:type="spellEnd"/></w:p>"#,
        "<?x-processing keep?>",
        r#"<w:p><w:r><w:t>Last paragraph.</w:t></w:r></w:p>"#
    )
    .to_string()
}

fn settings(f: Fixture, inner: &str) -> Fixture {
    f.rel("rIdSettings", "settings", "settings.xml").part(
        "word/settings.xml",
        Some("application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml"),
        format!(r#"{DECL}<w:settings {NS}>{inner}</w:settings>"#),
    )
}

/// A theme whose major font is Georgia and minor font Verdana.
fn theme(f: Fixture) -> Fixture {
    let colour =
        |name: &str, rgb: &str| format!(r#"<a:{name}><a:srgbClr val="{rgb}"/></a:{name}>"#);
    let colours: String = [
        ("dk2", "44546A"),
        ("lt2", "E7E6E6"),
        ("accent1", "4472C4"),
        ("accent2", "ED7D31"),
        ("accent3", "A5A5A5"),
        ("accent4", "FFC000"),
        ("accent5", "5B9BD5"),
        ("accent6", "70AD47"),
        ("hlink", "0563C1"),
        ("folHlink", "954F72"),
    ]
    .iter()
    .map(|(n, c)| colour(n, c))
    .collect();
    let fill = r#"<a:solidFill><a:schemeClr val="phClr"/></a:solidFill>"#;
    let line = format!(r#"<a:ln w="6350">{fill}</a:ln>"#);
    let effect = "<a:effectStyle><a:effectLst/></a:effectStyle>";
    let theme = format!(
        concat!(
            r#"{DECL}<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" name="Fixture"><a:themeElements>"#,
            r#"<a:clrScheme name="Fixture"><a:dk1><a:sysClr val="windowText" lastClr="000000"/></a:dk1><a:lt1><a:sysClr val="window" lastClr="FFFFFF"/></a:lt1>{colours}</a:clrScheme>"#,
            r#"<a:fontScheme name="Fixture"><a:majorFont><a:latin typeface="Georgia"/><a:ea typeface=""/><a:cs typeface=""/></a:majorFont><a:minorFont><a:latin typeface="Verdana"/><a:ea typeface=""/><a:cs typeface=""/></a:minorFont></a:fontScheme>"#,
            r#"<a:fmtScheme name="Fixture"><a:fillStyleLst>{fill}{fill}{fill}</a:fillStyleLst><a:lnStyleLst>{line}{line}{line}</a:lnStyleLst><a:effectStyleLst>{effect}{effect}{effect}</a:effectStyleLst><a:bgFillStyleLst>{fill}{fill}{fill}</a:bgFillStyleLst></a:fmtScheme>"#,
            r#"</a:themeElements><a:objectDefaults/><a:extraClrSchemeLst/></a:theme>"#
        ),
        DECL = DECL,
        colours = colours,
        fill = fill,
        line = line,
        effect = effect,
    );
    f.rel("rIdTheme", "theme", "theme/theme1.xml").part(
        "word/theme/theme1.xml",
        Some("application/vnd.openxmlformats-officedocument.theme+xml"),
        theme,
    )
}

/// Sentences enough to fill pages, the same on every run.
fn prose(paragraphs: usize) -> String {
    const SENTENCES: [&str; 6] = [
        "The engine lays out each paragraph from the properties its styles and the document give it.",
        "Lines break where the next word no longer fits between the indents.",
        "A page ends when the next line would cross the bottom margin, unless a paragraph asks to stay together.",
        "Headers and footers repeat on every page of their section, and fields in them show the page number.",
        "Tables take their borders from their style, then from the table, then from each cell.",
        "Footnotes are numbered in the order the body refers to them.",
    ];
    (0..paragraphs)
        .map(|i| {
            let text: Vec<&str> = (0..5)
                .map(|j| SENTENCES[(i + j) % SENTENCES.len()])
                .collect();
            p(&text.join(" "))
        })
        .collect()
}

fn notes() -> Fixture {
    let reference = |kind: &str, id: u32| {
        format!(
            r#"<w:r><w:rPr><w:vertAlign w:val="superscript"/></w:rPr><w:{kind}Reference w:id="{id}"/></w:r>"#
        )
    };
    let note = |kind: &str, id: u32, text: &str| {
        format!(
            r#"<w:{kind} w:id="{id}"><w:p><w:r><w:rPr><w:vertAlign w:val="superscript"/></w:rPr><w:{kind}Ref/></w:r><w:r><w:t xml:space="preserve"> {text}</w:t></w:r></w:p></w:{kind}>"#
        )
    };
    let separators = |kind: &str| {
        format!(
            r#"<w:{kind} w:type="separator" w:id="-1"><w:p><w:pPr><w:spacing w:after="0" w:line="240" w:lineRule="auto"/></w:pPr><w:r><w:separator/></w:r></w:p></w:{kind}><w:{kind} w:type="continuationSeparator" w:id="0"><w:p><w:pPr><w:spacing w:after="0" w:line="240" w:lineRule="auto"/></w:pPr><w:r><w:continuationSeparator/></w:r></w:p></w:{kind}>"#
        )
    };
    let body = format!(
        concat!(
            r#"<w:p><w:r><w:t>A claim</w:t></w:r>{f1}<w:r><w:t xml:space="preserve"> and another</w:t></w:r>{f2}<w:r><w:t>.</w:t></w:r></w:p>"#,
            r#"<w:p><w:r><w:t>A custom mark</w:t></w:r><w:r><w:rPr><w:vertAlign w:val="superscript"/></w:rPr><w:footnoteReference w:customMarkFollows="1" w:id="3"/><w:t>*</w:t></w:r><w:r><w:t xml:space="preserve"> takes no number.</w:t></w:r></w:p>"#,
            r#"<w:p><w:r><w:t>The next footnote</w:t></w:r>{f4}<w:r><w:t xml:space="preserve"> is the third, and an endnote follows</w:t></w:r>{e1}<w:r><w:t>.</w:t></w:r></w:p>"#,
            "{sect}"
        ),
        f1 = reference("footnote", 1),
        f2 = reference("footnote", 2),
        f4 = reference("footnote", 4),
        e1 = reference("endnote", 1),
        sect = SECT,
    );
    let footnotes = format!(
        r#"{DECL}<w:footnotes {NS}>{}{}{}<w:footnote w:id="3"><w:p><w:r><w:rPr><w:vertAlign w:val="superscript"/></w:rPr><w:t>*</w:t></w:r><w:r><w:t xml:space="preserve"> A note with its own mark.</w:t></w:r></w:p></w:footnote>{}</w:footnotes>"#,
        separators("footnote"),
        note("footnote", 1, "The first footnote."),
        note("footnote", 2, "The second footnote."),
        note("footnote", 4, "The third numbered footnote."),
    );
    let endnotes = format!(
        r#"{DECL}<w:endnotes {NS}>{}{}</w:endnotes>"#,
        separators("endnote"),
        note("endnote", 1, "An endnote, numbered from IV."),
    );
    settings(
        Fixture::new(body),
        concat!(
            r#"<w:footnotePr><w:footnote w:id="-1"/><w:footnote w:id="0"/></w:footnotePr>"#,
            r#"<w:endnotePr><w:numFmt w:val="upperRoman"/><w:numStart w:val="4"/><w:endnote w:id="-1"/><w:endnote w:id="0"/></w:endnotePr>"#
        ),
    )
    .rel("rIdFootnotes", "footnotes", "footnotes.xml")
    .part(
        "word/footnotes.xml",
        Some("application/vnd.openxmlformats-officedocument.wordprocessingml.footnotes+xml"),
        footnotes,
    )
    .rel("rIdEndnotes", "endnotes", "endnotes.xml")
    .part(
        "word/endnotes.xml",
        Some("application/vnd.openxmlformats-officedocument.wordprocessingml.endnotes+xml"),
        endnotes,
    )
}

/// Fields Word updates: a table of contents in the content control Word
/// puts it in, a date, the page and page count, and a page reference, all
/// with stale results. The front matter is numbered in roman, and each
/// heading starts a page.
fn fields() -> String {
    let run = |text: &str| format!(r#"<w:r><w:t xml:space="preserve">{text}</w:t></w:r>"#);
    let field = |instruction: &str, result: &str| {
        format!(
            r#"<w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve"> {instruction} </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>{result}</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r>"#
        )
    };
    let entry = |begin: &str, anchor: &str, text: &str, end: &str| {
        format!(
            r#"<w:p><w:pPr><w:tabs><w:tab w:val="right" w:leader="dot" w:pos="9350"/></w:tabs></w:pPr>{begin}<w:hyperlink w:anchor="{anchor}" w:history="1">{}<w:r><w:tab/></w:r>{}</w:hyperlink>{end}</w:p>"#,
            run(text),
            field(&format!(r"PAGEREF {anchor} \h"), "9"),
        )
    };
    let toc_begin = r#"<w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve"> TOC \o "1-3" \h \z \u </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r>"#;
    let toc_end = r#"<w:r><w:fldChar w:fldCharType="end"/></w:r>"#;
    let heading = |id: u32, text: &str| {
        format!(
            r#"<w:p><w:pPr><w:pStyle w:val="Heading1"/><w:pageBreakBefore/></w:pPr><w:bookmarkStart w:id="{id}" w:name="_Toc10{id}"/>{}<w:bookmarkEnd w:id="{id}"/></w:p>"#,
            run(text)
        )
    };
    let section = |numbers: &str| {
        format!(
            r#"<w:sectPr><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="720" w:footer="720" w:gutter="0"/><w:pgNumType {numbers} w:start="1"/></w:sectPr>"#
        )
    };
    [
        styled("Title", "Fields"),
        format!(
            r#"<w:sdt><w:sdtPr><w:id w:val="201"/><w:docPartObj><w:docPartGallery w:val="Table of Contents"/><w:docPartUnique/></w:docPartObj></w:sdtPr><w:sdtContent>{}{}{}{}</w:sdtContent></w:sdt>"#,
            p("Contents"),
            entry(toc_begin, "_Toc100", "Introduction", ""),
            entry("", "_Toc101", "Method", ""),
            entry("", "_Toc102", "Results", toc_end),
        ),
        format!(
            "<w:p>{}{}</w:p>",
            run("Printed on "),
            field(r#"DATE \@ "dddd, MMMM d, yyyy""#, "Monday, January 1, 2024")
        ),
        format!(
            "<w:p><w:pPr>{}</w:pPr>{}{}{}{}{}</w:p>",
            section(r#"w:fmt="lowerRoman""#),
            run("This is page "),
            field(r"PAGE \* MERGEFORMAT", "9"),
            run(" of "),
            field("NUMPAGES", "9"),
            run("."),
        ),
        heading(0, "Introduction"),
        format!(
            "<w:p>{}{}{}</w:p>",
            run("The method is on page "),
            field(r"PAGEREF _Toc101 \h", "9"),
            run("."),
        ),
        heading(1, "Method"),
        p("How it was done."),
        heading(2, "Results"),
        p("What was found."),
        section(r#"w:fmt="decimal""#),
    ]
    .concat()
}

fn sections() -> Fixture {
    const HEADER: &str =
        "application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml";
    const FOOTER: &str =
        "application/vnd.openxmlformats-officedocument.wordprocessingml.footer+xml";
    let header = |text: &str| {
        format!(r#"{DECL}<w:hdr {NS}><w:p><w:r><w:t>{text}</w:t></w:r></w:p></w:hdr>"#)
    };
    let field = |instruction: &str, result: &str| {
        format!(
            r#"<w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve"> {instruction} </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>{result}</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r>"#
        )
    };
    let footer = |side: &str| {
        format!(
            r#"{DECL}<w:ftr {NS}><w:p><w:pPr><w:jc w:val="{side}"/></w:pPr><w:r><w:t xml:space="preserve">Page </w:t></w:r>{}<w:r><w:t xml:space="preserve"> of </w:t></w:r>{}</w:p></w:ftr>"#,
            field("PAGE", "1"),
            field("NUMPAGES", "4"),
        )
    };
    let body = format!(
        concat!(
            r#"<w:p><w:pPr><w:pStyle w:val="Title"/></w:pPr><w:r><w:t>Sections</w:t></w:r></w:p>"#,
            "{first}",
            r#"<w:p><w:pPr><w:sectPr><w:headerReference w:type="default" r:id="rIdHeaderDefault"/><w:headerReference w:type="first" r:id="rIdHeaderFirst"/><w:headerReference w:type="even" r:id="rIdHeaderEven"/><w:footerReference w:type="default" r:id="rIdFooterOdd"/><w:footerReference w:type="even" r:id="rIdFooterEven"/><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="720" w:footer="720" w:gutter="0"/><w:pgNumType w:fmt="lowerRoman" w:start="1"/><w:titlePg/></w:sectPr></w:pPr><w:r><w:t>Last paragraph of the portrait section.</w:t></w:r></w:p>"#,
            "{landscape}",
            r#"<w:p><w:pPr><w:sectPr><w:type w:val="nextPage"/><w:pgSz w:w="15840" w:h="12240" w:orient="landscape"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="720" w:footer="720" w:gutter="0"/></w:sectPr></w:pPr><w:r><w:t>Last paragraph of the landscape section.</w:t></w:r></w:p>"#,
            "{columns}",
            r#"<w:sectPr><w:type w:val="continuous"/><w:pgSz w:w="15840" w:h="12240" w:orient="landscape"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="720" w:footer="720" w:gutter="0"/><w:cols w:num="2" w:space="708"/></w:sectPr>"#
        ),
        first = prose(14),
        landscape = prose(6),
        columns = prose(8),
    );
    settings(
        Fixture::new(body),
        r#"<w:defaultTabStop w:val="720"/><w:evenAndOddHeaders/>"#,
    )
    .rel("rIdHeaderDefault", "header", "header1.xml")
    .rel("rIdHeaderFirst", "header", "header2.xml")
    .rel("rIdHeaderEven", "header", "header3.xml")
    .rel("rIdFooterOdd", "footer", "footer1.xml")
    .rel("rIdFooterEven", "footer", "footer2.xml")
    .part("word/header1.xml", Some(HEADER), header("Odd page header"))
    .part(
        "word/header2.xml",
        Some(HEADER),
        header("Title page header"),
    )
    .part("word/header3.xml", Some(HEADER), header("Even page header"))
    .part("word/footer1.xml", Some(FOOTER), footer("right"))
    .part("word/footer2.xml", Some(FOOTER), footer("left"))
}

fn paragraph_layout() -> Fixture {
    let styles = format!(
        concat!(
            r#"{DECL}<w:styles {NS}><w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:asciiTheme="minorHAnsi" w:hAnsiTheme="minorHAnsi"/><w:sz w:val="22"/></w:rPr></w:rPrDefault><w:pPrDefault><w:pPr><w:spacing w:after="160" w:line="259" w:lineRule="auto"/></w:pPr></w:pPrDefault></w:docDefaults>"#,
            r#"<w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style>"#,
            r#"<w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/><w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:pPr><w:keepNext/><w:keepLines/><w:spacing w:before="480" w:after="120"/><w:outlineLvl w:val="0"/></w:pPr><w:rPr><w:rFonts w:asciiTheme="majorHAnsi" w:hAnsiTheme="majorHAnsi"/><w:sz w:val="32"/></w:rPr></w:style>"#,
            r#"<w:style w:type="paragraph" w:styleId="ListParagraph"><w:name w:val="List Paragraph"/><w:basedOn w:val="Normal"/><w:pPr><w:ind w:left="1080"/><w:contextualSpacing/></w:pPr></w:style>"#,
            r#"<w:style w:type="paragraph" w:styleId="ListNumber"><w:name w:val="List Number"/><w:basedOn w:val="Normal"/><w:pPr><w:numPr><w:numId w:val="1"/></w:numPr><w:ind w:left="2160"/></w:pPr></w:style>"#,
            r#"<w:style w:type="paragraph" w:styleId="TabStops"><w:name w:val="Tab Stops"/><w:basedOn w:val="Normal"/><w:pPr><w:tabs><w:tab w:val="left" w:pos="1440"/><w:tab w:val="center" w:pos="4680"/></w:tabs></w:pPr></w:style>"#,
            r#"</w:styles>"#
        ),
        DECL = DECL,
        NS = NS,
    );
    let numbering = format!(
        r#"{DECL}<w:numbering {NS}><w:abstractNum w:abstractNumId="1"><w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="decimal"/><w:suff w:val="tab"/><w:lvlText w:val="%1."/><w:lvlJc w:val="left"/><w:pPr><w:ind w:left="720" w:hanging="360"/></w:pPr><w:rPr><w:b/></w:rPr></w:lvl><w:lvl w:ilvl="1"><w:start w:val="1"/><w:numFmt w:val="lowerLetter"/><w:suff w:val="space"/><w:lvlText w:val="%2)"/><w:lvlJc w:val="right"/><w:pPr><w:ind w:left="1440" w:hanging="360"/></w:pPr></w:lvl></w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="1"/></w:num></w:numbering>"#
    );
    let para = |ppr: &str, text: &str| {
        format!(r#"<w:p><w:pPr>{ppr}</w:pPr><w:r><w:t>{text}</w:t></w:r></w:p>"#)
    };
    let long = "This paragraph runs long enough to wrap across several lines, so its first-line indent, its spacing and its alignment all show in the layout.";
    let body = [
        para(r#"<w:pStyle w:val="Heading1"/>"#, "Paragraph layout"),
        para(
            r#"<w:spacing w:before="120" w:after="240" w:line="360" w:lineRule="auto"/><w:ind w:firstLine="720"/><w:jc w:val="both"/>"#,
            long,
        ),
        para(r#"<w:spacing w:line="300" w:lineRule="exact"/><w:jc w:val="center"/>"#, "Exactly fifteen points per line, centred."),
        para(
            r#"<w:spacing w:line="480" w:lineRule="atLeast"/><w:ind w:left="1440" w:right="720" w:firstLine="200" w:hanging="360"/><w:jc w:val="right"/>"#,
            long,
        ),
        para(r#"<w:pStyle w:val="ListParagraph"/>"#, "Contextual spacing joins this paragraph"),
        para(r#"<w:pStyle w:val="ListParagraph"/>"#, "to this one in the same style."),
        r#"<w:p><w:pPr><w:pStyle w:val="TabStops"/><w:tabs><w:tab w:val="clear" w:pos="4680"/><w:tab w:val="center" w:pos="4320"/><w:tab w:val="decimal" w:pos="7200"/><w:tab w:val="right" w:leader="dot" w:pos="9360"/></w:tabs></w:pPr><w:r><w:t>Left</w:t><w:tab/><w:t>Centre</w:t><w:tab/><w:t>12.50</w:t><w:tab/><w:t>Right</w:t></w:r></w:p>"#.to_string(),
        para(r#"<w:keepNext/>"#, "Kept with the next paragraph."),
        para(r#"<w:pageBreakBefore/>"#, "Starts on a new page."),
        para(r#"<w:numPr><w:ilvl w:val="0"/><w:numId w:val="1"/></w:numPr>"#, "A numbered item indented by its level"),
        para(r#"<w:numPr><w:ilvl w:val="1"/><w:numId w:val="1"/></w:numPr>"#, "A lettered sub-item"),
        para(
            r#"<w:pStyle w:val="ListParagraph"/><w:numPr><w:ilvl w:val="0"/><w:numId w:val="1"/></w:numPr>"#,
            "Direct numbering overrides the style's indent",
        ),
        para(r#"<w:pStyle w:val="ListNumber"/>"#, "The style's indent overrides numbering it applies"),
        para(r#"<w:spacing w:before="100" w:beforeAutospacing="1"/>"#, "Automatic spacing before, as HTML has it."),
        r#"<w:p><w:r><w:t xml:space="preserve">Ordinary text and </w:t></w:r><w:r><w:rPr><w:smallCaps/></w:rPr><w:t>small capitals</w:t></w:r><w:r><w:t>.</w:t></w:r></w:p>"#.to_string(),
        r#"<w:p><w:pPr><w:rPr><w:sz w:val="40"/></w:rPr></w:pPr></w:p>"#.to_string(),
        SECT.to_string(),
    ]
    .concat();
    theme(settings(
        Fixture::new(body)
            .with_styles(styles)
            .with_numbering_xml(numbering),
        r#"<w:defaultTabStop w:val="709"/>"#,
    ))
}

fn table_layout() -> Fixture {
    let styles = format!(
        concat!(
            r#"{DECL}<w:styles {NS}><w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii="Calibri" w:hAnsi="Calibri"/><w:sz w:val="22"/></w:rPr></w:rPrDefault><w:pPrDefault><w:pPr><w:spacing w:after="160" w:line="259" w:lineRule="auto"/></w:pPr></w:pPrDefault></w:docDefaults>"#,
            r#"<w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/><w:rPr><w:color w:val="333333"/></w:rPr></w:style>"#,
            r#"<w:style w:type="paragraph" w:styleId="Body"><w:name w:val="Body"/><w:basedOn w:val="Normal"/></w:style>"#,
            r#"<w:style w:type="table" w:default="1" w:styleId="TableNormal"><w:name w:val="Normal Table"/><w:uiPriority w:val="99"/><w:semiHidden/><w:unhideWhenUsed/><w:tblPr><w:tblInd w:w="0" w:type="dxa"/><w:tblCellMar><w:top w:w="0" w:type="dxa"/><w:left w:w="108" w:type="dxa"/><w:bottom w:w="0" w:type="dxa"/><w:right w:w="108" w:type="dxa"/></w:tblCellMar></w:tblPr></w:style>"#,
            r#"<w:style w:type="table" w:styleId="TableGrid"><w:name w:val="Table Grid"/><w:basedOn w:val="TableNormal"/><w:uiPriority w:val="39"/><w:pPr><w:spacing w:after="0" w:line="240" w:lineRule="auto"/></w:pPr><w:tblPr><w:tblBorders><w:top w:val="single" w:sz="4" w:space="0" w:color="auto"/><w:left w:val="single" w:sz="4" w:space="0" w:color="auto"/><w:bottom w:val="single" w:sz="4" w:space="0" w:color="auto"/><w:right w:val="single" w:sz="4" w:space="0" w:color="auto"/><w:insideH w:val="single" w:sz="4" w:space="0" w:color="auto"/><w:insideV w:val="single" w:sz="4" w:space="0" w:color="auto"/></w:tblBorders></w:tblPr></w:style>"#,
            r#"<w:style w:type="table" w:styleId="Accent"><w:name w:val="Accent"/><w:basedOn w:val="TableGrid"/><w:rPr><w:color w:val="C00000"/></w:rPr><w:tblPr><w:jc w:val="center"/><w:tblCellMar><w:left w:w="144" w:type="dxa"/><w:right w:w="144" w:type="dxa"/></w:tblCellMar></w:tblPr><w:tcPr><w:shd w:val="clear" w:color="auto" w:fill="E2EFDA"/></w:tcPr></w:style>"#,
            r#"</w:styles>"#
        ),
        DECL = DECL,
        NS = NS,
    );
    let cell = |tc_pr: &str, text: &str| {
        let content = if text.is_empty() {
            "<w:p/>".to_string()
        } else {
            format!(r#"<w:p><w:r><w:t>{text}</w:t></w:r></w:p>"#)
        };
        format!(r#"<w:tc><w:tcPr><w:tcW w:w="3000" w:type="dxa"/>{tc_pr}</w:tcPr>{content}</w:tc>"#)
    };
    let header_fill = r#"<w:shd w:val="clear" w:color="auto" w:fill="D9E2F3"/>"#;
    let grid = r#"<w:tblGrid><w:gridCol w:w="3000"/><w:gridCol w:w="3000"/><w:gridCol w:w="3000"/></w:tblGrid>"#;
    let first = format!(
        concat!(
            r#"<w:tbl><w:tblPr><w:tblStyle w:val="TableGrid"/><w:tblW w:w="0" w:type="auto"/><w:tblInd w:w="144" w:type="dxa"/><w:tblBorders><w:top w:val="single" w:sz="12" w:space="0" w:color="auto"/><w:bottom w:val="single" w:sz="12" w:space="0" w:color="auto"/><w:insideV w:val="dotted" w:sz="4" w:space="0" w:color="808080"/></w:tblBorders><w:tblLook w:val="04A0" w:firstRow="1" w:lastRow="0" w:firstColumn="1" w:lastColumn="0" w:noHBand="0" w:noVBand="1"/></w:tblPr>{grid}"#,
            r#"<w:tr><w:trPr><w:cantSplit/><w:tblHeader/></w:trPr>{h1}{h2}{h3}</w:tr>"#,
            r#"<w:tr><w:trPr><w:trHeight w:val="720" w:hRule="exact"/></w:trPr>{merge}{bordered}{padded}</w:tr>"#,
            r#"<w:tr><w:trPr><w:trHeight w:val="1080" w:hRule="atLeast"/></w:trPr>{merged}{bottom}{centre}</w:tr>"#,
            r#"<w:tr><w:trPr><w:gridBefore w:val="1"/><w:wBefore w:w="3000" w:type="dxa"/></w:trPr><w:tc><w:tcPr><w:tcW w:w="6000" w:type="dxa"/><w:gridSpan w:val="2"/></w:tcPr><w:p><w:r><w:t>After a skipped column</w:t></w:r></w:p></w:tc></w:tr>"#,
            r#"</w:tbl>"#
        ),
        grid = grid,
        h1 = cell(header_fill, "Header A"),
        h2 = cell(header_fill, "Header B"),
        h3 = cell(header_fill, "Header C"),
        merge = cell(r#"<w:vMerge w:val="restart"/>"#, "Merged down"),
        bordered = cell(
            r#"<w:tcBorders><w:top w:val="double" w:sz="12" w:space="0" w:color="FF0000"/><w:bottom w:val="nil"/></w:tcBorders>"#,
            "Own borders",
        ),
        padded = cell(
            r#"<w:tcMar><w:top w:w="120" w:type="dxa"/><w:left w:w="360" w:type="dxa"/><w:right w:w="360" w:type="dxa"/></w:tcMar>"#,
            "Padded",
        ),
        merged = cell("<w:vMerge/>", ""),
        bottom = cell(r#"<w:vAlign w:val="bottom"/>"#, "Bottom aligned"),
        centre = cell(r#"<w:vAlign w:val="center"/>"#, "Centred"),
    );
    let second = format!(
        concat!(
            r#"<w:tbl><w:tblPr><w:tblStyle w:val="Accent"/><w:tblW w:w="0" w:type="auto"/><w:tblLook w:val="04A0" w:firstRow="1" w:lastRow="0" w:firstColumn="1" w:lastColumn="0" w:noHBand="0" w:noVBand="1"/></w:tblPr><w:tblGrid><w:gridCol w:w="3000"/><w:gridCol w:w="3000"/></w:tblGrid>"#,
            r#"<w:tr>{styled}<w:tc><w:tcPr><w:tcW w:w="3000" w:type="dxa"/><w:shd w:val="clear" w:color="auto" w:fill="FFF2CC"/></w:tcPr><w:p><w:pPr><w:pStyle w:val="Body"/></w:pPr><w:r><w:t>Own fill, chosen style</w:t></w:r></w:p></w:tc></w:tr>"#,
            r#"</w:tbl>"#
        ),
        styled = cell("", "Style fill, default style"),
    );
    Fixture::new(format!(
        "{}{first}{}{second}{}{SECT}",
        p("Before the tables."),
        p("Between the tables."),
        p("After the tables.")
    ))
    .with_styles(styles)
}

fn anchored_image() -> Fixture {
    let graphic = |id: u32| {
        format!(
            r#"<a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic><pic:nvPicPr><pic:cNvPr id="{id}" name="image1.png"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed="rIdImage1"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill><pic:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="1828800" cy="1371600"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></pic:spPr></pic:pic></a:graphicData></a:graphic>"#
        )
    };
    let square = format!(
        concat!(
            r#"<w:r><w:drawing><wp:anchor distT="0" distB="0" distL="114300" distR="114300" simplePos="0" relativeHeight="251659264" behindDoc="0" locked="0" layoutInCell="1" allowOverlap="1">"#,
            r#"<wp:simplePos x="0" y="0"/><wp:positionH relativeFrom="column"><wp:posOffset>3657600</wp:posOffset></wp:positionH><wp:positionV relativeFrom="paragraph"><wp:align>top</wp:align></wp:positionV>"#,
            r#"<wp:extent cx="1828800" cy="1371600"/><wp:effectExtent l="0" t="0" r="0" b="0"/><wp:wrapSquare wrapText="bothSides"/><wp:docPr id="2" name="Picture 2" descr="Wrapped by text"/><wp:cNvGraphicFramePr/>{}</wp:anchor></w:drawing></w:r>"#
        ),
        graphic(2)
    );
    let behind = format!(
        concat!(
            r#"<w:r><w:drawing><wp:anchor distT="0" distB="0" distL="0" distR="0" simplePos="0" relativeHeight="251658240" behindDoc="1" locked="0" layoutInCell="1" allowOverlap="1">"#,
            r#"<wp:simplePos x="0" y="0"/><wp:positionH relativeFrom="page"><wp:align>center</wp:align></wp:positionH><wp:positionV relativeFrom="page"><wp:posOffset>914400</wp:posOffset></wp:positionV>"#,
            r#"<wp:extent cx="1828800" cy="1371600"/><wp:effectExtent l="0" t="0" r="0" b="0"/><wp:wrapNone/><wp:docPr id="3" name="Picture 3" descr="Behind the text"/><wp:cNvGraphicFramePr/>{}</wp:anchor></w:drawing></w:r>"#
        ),
        graphic(3)
    );
    let long = "Text flows around the picture on its left, line after line, until the paragraph passes below the picture's bottom edge and the lines take the whole width again. ";
    Fixture::new(format!(
        r#"{}<w:p>{square}<w:r><w:t xml:space="preserve">{}</w:t></w:r></w:p><w:p>{behind}<w:r><w:t>A picture behind this text, centred on the page.</w:t></w:r></w:p><w:p>{}</w:p>{}{SECT}"#,
        p("An anchored picture follows."),
        long.repeat(4),
        image_run("rIdImage1"),
        p("Text after the inline picture.")
    ))
    .rel("rIdImage1", "image", "media/image1.png")
    .part("word/media/image1.png", None, PNG)
}

fn main() {
    let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/generated");
    std::fs::create_dir_all(&out).unwrap();

    let fixtures: Vec<(&str, Fixture)> = vec![
        (
            "simple-paragraphs",
            Fixture::new(format!("{}{}{}{SECT}", p("First paragraph."), p("Second paragraph."), p("Third paragraph."))),
        ),
        (
            "rich-text",
            Fixture::new(format!(
                concat!(
                    r#"<w:p w14:paraId="1A2B3C4D" w14:textId="77777777" w:rsidR="00AB12CD" w:rsidRDefault="00AB12CD">"#,
                    r#"<w:r><w:rPr><w:b/></w:rPr><w:t>Bold</w:t></w:r><w:r><w:t xml:space="preserve"> and </w:t></w:r>"#,
                    r#"<w:r><w:rPr><w:i/></w:rPr><w:t>italic</w:t></w:r><w:r><w:t xml:space="preserve"> and </w:t></w:r>"#,
                    r#"<w:r><w:rPr><w:u w:val="single"/></w:rPr><w:t>underlined</w:t></w:r><w:r><w:t>.</w:t></w:r></w:p>"#,
                    r#"<w:p><w:r><w:rPr><w:rFonts w:ascii="Georgia" w:hAnsi="Georgia"/><w:color w:val="C00000"/><w:sz w:val="28"/></w:rPr><w:t>Coloured text</w:t></w:r>"#,
                    r#"<w:r><w:t xml:space="preserve">  leading and trailing spaces  </w:t></w:r></w:p>"#,
                    r#"<w:p><w:r><w:t>Ampersand &amp; angle &lt;brackets&gt; "quotes"</w:t></w:r></w:p>"#,
                    "<w:p><w:r><w:t>Emoji 😀, CJK 漢字, combining e\u{301}</w:t></w:r></w:p>",
                    r#"<w:p><w:r><w:t>Before tab</w:t><w:tab/><w:t>after tab</w:t><w:br/><w:t>next line</w:t></w:r></w:p>"#,
                    "{}"
                ),
                SECT
            )),
        ),
        (
            "headings-styles",
            Fixture::new(format!(
                "{}{}{}{}{}{}{SECT}",
                styled("Title", "Document title"),
                styled("Heading1", "Chapter one"),
                p("Body text under chapter one."),
                styled("Heading2", "Section 1.1"),
                r#"<w:p><w:r><w:t xml:space="preserve">Plain then </w:t></w:r><w:r><w:rPr><w:rStyle w:val="Emphasis"/></w:rPr><w:t>emphasised</w:t></w:r><w:r><w:t>.</w:t></w:r></w:p>"#,
                r#"<w:p><w:pPr><w:outlineLvl w:val="2"/></w:pPr><w:r><w:t>Direct outline level three</w:t></w:r></w:p>"#,
            )),
        ),
        (
            "lists",
            Fixture::new(format!(
                "{}{}{}{}{}{}{}{SECT}",
                list(1, 0, "Bullet one"),
                list(1, 1, "Nested bullet"),
                list(1, 0, "Bullet two"),
                list(2, 0, "Step one"),
                list(2, 1, "Sub-step a"),
                styled("ListBullet", "Bullet from the style"),
                r#"<w:p><w:pPr><w:pStyle w:val="ListBullet"/><w:numPr><w:numId w:val="0"/></w:numPr></w:pPr><w:r><w:t>Style bullet removed by numId 0</w:t></w:r></w:p>"#,
            ))
            .with_numbering(),
        ),
        ("table", Fixture::new(format!("{}{}{}{SECT}", p("Before the table."), table(), p("After the table.")))),
        (
            "image",
            Fixture::new(format!(
                r#"{}<w:p>{}</w:p>{}{SECT}"#,
                p("An image follows."),
                image_run("rIdImage1"),
                p("Text after the image.")
            ))
            .rel("rIdImage1", "image", "media/image1.png")
            .part("word/media/image1.png", None, PNG),
        ),
        (
            "header-footer",
            header_footer(Fixture::new(format!("{}{}{SECT_HF}", p("Body with a header and footer."), p("Second body paragraph.")))),
        ),
        ("tracked-changes", Fixture::new(format!("{}{SECT}", tracked()))),
        ("comments", comments()),
        ("unknown-ooxml", comments_part(Fixture::new(format!("{}{SECT}", unknown())))),
        (
            "complex-mixed",
            comments_part(header_footer(
                Fixture::new(format!(
                    concat!(
                        "{title}",
                        r#"<w:p><w:pPr><w:pStyle w:val="Normal"/></w:pPr><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve"> TOC \o "1-3" </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>Chapter one ... 1</w:t></w:r></w:p>"#,
                        r#"<w:p><w:r><w:t>Chapter two ... 2</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r></w:p>"#,
                        "{h1}",
                        r#"<w:p><w:r><w:t xml:space="preserve">See </w:t></w:r><w:hyperlink r:id="rIdLink"><w:r><w:rPr><w:u w:val="single"/></w:rPr><w:t>the example site</w:t></w:r></w:hyperlink><w:r><w:t xml:space="preserve"> for details.</w:t></w:r><w:r><w:footnoteReference w:id="1"/></w:r></w:p>"#,
                        "{l1}{l2}{l3}",
                        "{table}",
                        r#"<w:p>{img}</w:p>"#,
                        "{tracked}",
                        r#"<w:p><w:pPr><w:sectPr><w:pgSz w:w="15840" w:h="12240" w:orient="landscape"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="720" w:footer="720" w:gutter="0"/></w:sectPr></w:pPr><w:r><w:t>Last paragraph of the landscape section.</w:t></w:r></w:p>"#,
                        "{h2}",
                        r#"<x:customBlock><x:inner>unknown block</x:inner></x:customBlock>"#,
                        r#"<w:p><w:bookmarkStart w:id="0" w:name="end"/><w:commentRangeStart w:id="0"/><w:r><w:t>Closing paragraph with a comment.</w:t></w:r><w:commentRangeEnd w:id="0"/><w:r><w:commentReference w:id="0"/></w:r><w:bookmarkEnd w:id="0"/></w:p>"#,
                        "{sect}"
                    ),
                    title = styled("Title", "Mixed document"),
                    h1 = styled("Heading1", "Chapter one"),
                    l1 = list(2, 0, "First point"),
                    l2 = list(2, 1, "Detail"),
                    l3 = list(1, 0, "A bullet"),
                    table = table(),
                    img = image_run("rIdImage1"),
                    tracked = tracked(),
                    h2 = styled("Heading2", "Appendix"),
                    sect = SECT_HF,
                ))
                .with_numbering()
                .rel("rIdImage1", "image", "media/image1.png")
                .part("word/media/image1.png", None, PNG)
                .external("rIdLink", "hyperlink", "https://example.com/")
                .rel("rIdFootnotes", "footnotes", "footnotes.xml")
                .part(
                    "word/footnotes.xml",
                    Some("application/vnd.openxmlformats-officedocument.wordprocessingml.footnotes+xml"),
                    format!(
                        r#"{DECL}<w:footnotes {NS}><w:footnote w:type="separator" w:id="-1"><w:p><w:r><w:separator/></w:r></w:p></w:footnote><w:footnote w:id="1"><w:p><w:r><w:t>A footnote the engine preserves.</w:t></w:r></w:p></w:footnote></w:footnotes>"#
                    ),
                ),
            )),
        ),
        ("notes", notes()),
        ("sections", sections()),
        ("paragraph-layout", paragraph_layout()),
        ("table-layout", table_layout()),
        ("anchored-image", anchored_image()),
        ("content-controls", Fixture::new(format!("{}{SECT}", content_controls()))),
        ("fields", Fixture::new(fields())),
    ];

    for (name, fixture) in fixtures {
        let path = out.join(format!("{name}.docx"));
        std::fs::write(&path, fixture.build()).unwrap();
        println!("{}", path.display());
    }
}
