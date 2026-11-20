use super::common;

use common::*;
use docx_engine::{Command, Document, DrawingInfo, EditError, NodeKind, Point};

const DOCUMENT: &str = "word/document.xml";
const RELS: &str = "word/_rels/document.xml.rels";
const CONTENT_TYPES: &str = "[Content_Types].xml";
const IMAGE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image";
const WP: &str = "http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing";

/// The one-pixel PNG of the `image` fixture.
fn png() -> Vec<u8> {
    entries(&generated("image"))
        .into_iter()
        .find(|(name, _)| name == "word/media/image1.png")
        .unwrap()
        .1
}

/// 64-bit FNV-1a, which names media and their relationships.
fn fnv(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn insert(at: Point, media: &str, id: Option<u32>) -> Command {
    Command::InsertPicture {
        to: None,
        at,
        media: media.to_string(),
        width_emu: 914400,
        height_emu: 457200,
        description: Some("A dot & more".into()),
        id,
    }
}

/// Every drawing in document order.
fn drawings(doc: &Document) -> Vec<DrawingInfo> {
    let roots: Vec<_> = doc.stories().map(|s| s.id).collect();
    roots
        .into_iter()
        .flat_map(|r| doc.descendants(r))
        .filter_map(|id| match &doc.node(id).unwrap().kind {
            NodeKind::Drawing(info) => Some(DrawingInfo::clone(info)),
            _ => None,
        })
        .collect()
}

fn ids(doc: &Document) -> Vec<Option<u32>> {
    drawings(doc).iter().map(|d| d.id).collect()
}

#[test]
fn a_picture_is_written_with_its_media_and_relationship() {
    let base = generated("simple-paragraphs");
    let mut doc = EditingDocument::open(&base).unwrap();
    let png = png();
    let media = doc.0.media_name(&png).unwrap();
    assert_eq!(media, format!("word/media/image-{:016x}.png", fnv(&png)));
    let text = text_node(&doc, "First paragraph.");
    doc.0
        .apply_all_with_media(&[insert(at(text, 5), &media, None)], &[&png])
        .unwrap();
    assert_eq!(doc.part_bytes(&media), Some(png.clone()));

    let out = doc.export().unwrap();
    assert_valid("simple-paragraphs", &out);
    let rel = format!("rIdP{:016x}", fnv(media.as_bytes()));
    let file = media.rsplit('/').next().unwrap();
    let xml = part(&out, DOCUMENT);
    let drawing = format!(
        r#"<w:drawing><wp:inline xmlns:wp="{WP}" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" distT="0" distB="0" distL="0" distR="0"><wp:extent cx="914400" cy="457200"/><wp:docPr id="1" name="Picture 1" descr="A dot &amp; more"/><wp:cNvGraphicFramePr><a:graphicFrameLocks noChangeAspect="1"/></wp:cNvGraphicFramePr><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic><pic:nvPicPr><pic:cNvPr id="0" name="{file}"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed="{rel}"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill><pic:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="914400" cy="457200"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></pic:spPr></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing>"#
    );
    assert!(
        xml.contains(&format!(
            r#"<w:r><w:t xml:space="preserve">First</w:t>{drawing}<w:t xml:space="preserve"> paragraph.</w:t></w:r>"#
        )),
        "{xml}"
    );
    let rels = part(&out, RELS);
    assert!(
        rels.contains(&format!(
            r#"<Relationship Id="{rel}" Type="{IMAGE}" Target="media/{file}"/>"#
        )),
        "{rels}"
    );
    let written = entries(&out)
        .into_iter()
        .find(|(n, _)| *n == media)
        .unwrap();
    assert_eq!(written.1, png);
    // The fixture types PNG files already.
    assert_eq!(part(&out, CONTENT_TYPES), part(&base, CONTENT_TYPES));
    assert_eq!(doc.dirty_parts(), [DOCUMENT, &media, RELS]);

    let reopened = Document::open(&out).unwrap();
    let [picture] = &drawings(&reopened)[..] else {
        panic!("{:?}", drawings(&reopened));
    };
    assert_eq!(picture.id, Some(1));
    assert_eq!(picture.name.as_deref(), Some("Picture 1"));
    assert_eq!(picture.description.as_deref(), Some("A dot & more"));
    assert_eq!(
        (picture.width_emu, picture.height_emu),
        (Some(914400), Some(457200))
    );
    assert_eq!(picture.media[0].part.as_deref(), Some(media.as_str()));
    assert_eq!(picture.media[0].content_type.as_deref(), Some("image/png"));
    assert_eq!(
        format!("{:?}", drawings(&doc)[0].media),
        format!("{:?}", picture.media)
    );
    assert_eq!(reopened.part_bytes(&media), Some(png));

    assert_editor_undo("simple-paragraphs", &base, &mut doc);
}

#[test]
fn media_gets_a_content_type_when_the_package_has_none_for_it() {
    let base = generated("simple-paragraphs");
    let mut doc = EditingDocument::open(&base).unwrap();
    let jpeg = [0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, b'J', b'F', b'I', b'F'];
    let gif = *b"GIF89a\x01\x00\x01\x00";
    // Added in the reverse of name order, so export has to sort them.
    let gif_name = doc.0.media_name(&gif).unwrap();
    let jpeg_name = doc.0.media_name(&jpeg).unwrap();
    assert!(jpeg_name < gif_name);
    assert_eq!(
        jpeg_name,
        format!("word/media/image-{:016x}.jpeg", fnv(&jpeg))
    );
    assert_eq!(gif_name, format!("word/media/image-{:016x}.gif", fnv(&gif)));
    assert_eq!(
        doc.0.media_name(b"GIF87a\x01\x00").unwrap(),
        format!("word/media/image-{:016x}.gif", fnv(b"GIF87a\x01\x00"))
    );
    let text = text_node(&doc, "Second paragraph.");
    doc.0
        .apply_all_with_media(&[insert(at(text, 0), &jpeg_name, None)], &[&jpeg])
        .unwrap();
    let text = text_node(&doc, "Second paragraph.");
    doc.0
        .apply_all_with_media(&[insert(at(text, 6), &gif_name, None)], &[&gif])
        .unwrap();
    assert_eq!(ids(&doc), [Some(1), Some(2)]);

    let out = doc.export().unwrap();
    assert_valid("simple-paragraphs", &out);
    // Media is written in name order, whatever order it was added in, so
    // replicas that added it in different orders write the same bytes.
    let mut named = [(&jpeg_name, "image/jpeg"), (&gif_name, "image/gif")];
    named.sort();
    let overrides: String = named
        .iter()
        .map(|(name, ct)| format!(r#"<Override PartName="/{name}" ContentType="{ct}"/>"#))
        .collect();
    let types = part(&out, CONTENT_TYPES);
    assert!(types.ends_with(&format!("{overrides}</Types>")), "{types}");
    assert_eq!(
        doc.dirty_parts(),
        [DOCUMENT, CONTENT_TYPES, named[0].0, named[1].0, RELS]
    );
    let reopened = Document::open(&out).unwrap();
    let types: Vec<_> = drawings(&reopened)
        .into_iter()
        .map(|d| d.media[0].content_type.clone().unwrap())
        .collect();
    assert_eq!(types, ["image/jpeg", "image/gif"]);

    // Undoing the second leaves the first's content type alone.
    assert!(doc.0.undo());
    let types = part(&doc.export().unwrap(), CONTENT_TYPES);
    assert!(
        types.contains(&jpeg_name) && !types.contains(&gif_name),
        "{types}"
    );
}

#[test]
fn media_and_its_relationship_are_written_once_while_a_picture_uses_them() {
    let base = generated("simple-paragraphs");
    let mut doc = EditingDocument::open(&base).unwrap();
    let media = doc.0.media_name(&png()).unwrap();
    assert_eq!(doc.0.media_name(&png()).unwrap(), media);
    // Added media no picture uses is not part of the document.
    assert!(doc.dirty_parts().is_empty());
    assert_eq!(entries(&doc.export().unwrap()), entries(&base));

    let first = text_node(&doc, "First paragraph.");
    let third = text_node(&doc, "Third paragraph.");
    let run = doc.node(first).unwrap().parent.unwrap();
    doc.0
        .apply_all_with_media(&[insert(Point::End { end: run }, &media, None)], &[&png()])
        .unwrap();
    doc.0
        .apply_all_with_media(&[insert(at(third, 0), &media, None)], &[&png()])
        .unwrap();
    let out = doc.export().unwrap();
    assert_valid("simple-paragraphs", &out);
    let names: Vec<String> = entries(&out).into_iter().map(|e| e.0).collect();
    assert_eq!(names.iter().filter(|n| **n == media).count(), 1);
    let rels = part(&out, RELS);
    assert_eq!(rels.matches(IMAGE).count(), 1, "{rels}");
    assert_eq!(ids(&doc), [Some(1), Some(2)]);
}

#[test]
fn a_picture_in_a_header_relates_from_the_header() {
    let base = generated("header-footer");
    let mut doc = EditingDocument::open(&base).unwrap();
    let media = doc.0.media_name(&png()).unwrap();
    let text = text_node(&doc, "Header text");
    doc.0
        .apply_all_with_media(&[insert(at(text, 6), &media, None)], &[&png()])
        .unwrap();
    let out = doc.export().unwrap();
    assert_valid("header-footer", &out);
    let rels = part(&out, "word/_rels/header1.xml.rels");
    assert!(
        rels.contains(&format!(
            r#"<Relationship Id="rIdP{:016x}" Type="{IMAGE}" Target="media/{}"/>"#,
            fnv(media.as_bytes()),
            media.rsplit('/').next().unwrap()
        )),
        "{rels}"
    );
    assert_eq!(
        doc.dirty_parts(),
        ["word/header1.xml", &media, "word/_rels/header1.xml.rels"]
    );
}

#[test]
fn a_story_in_another_folder_relates_to_the_media_by_a_relative_target() {
    let base = generated("header-footer");
    let header = part(&base, "word/header1.xml");
    let rels =
        part(&base, RELS).replace(r#"Target="header1.xml""#, r#"Target="parts/header1.xml""#);
    let types = part(&base, CONTENT_TYPES).replace("/word/header1.xml", "/word/parts/header1.xml");
    let moved = rewrite(&base, "word/header1.xml", None);
    let moved = rewrite(&moved, RELS, Some(rels.as_bytes()));
    let moved = rewrite(&moved, CONTENT_TYPES, Some(types.as_bytes()));
    let moved = with_entry(&moved, "word/parts/header1.xml", header.as_bytes());
    let mut doc = EditingDocument::open(&moved).unwrap();
    let media = doc.0.media_name(&png()).unwrap();
    let text = text_node(&doc, "Header text");
    doc.0
        .apply_all_with_media(&[insert(at(text, 6), &media, None)], &[&png()])
        .unwrap();
    let out = doc.export().unwrap();
    assert_valid("header-footer, moved", &out);
    let rels = part(&out, "word/parts/_rels/header1.xml.rels");
    assert!(
        rels.contains(&format!(
            r#"Target="../media/{}"/>"#,
            media.rsplit('/').next().unwrap()
        )),
        "{rels}"
    );
}

#[test]
fn a_picture_takes_a_drawing_id_no_other_drawing_has() {
    let base = generated("image");
    let mut doc = EditingDocument::open(&base).unwrap();
    assert_eq!(ids(&doc), [Some(1)]);
    let media = doc.0.media_name(&png()).unwrap();
    let p = Point::Start {
        start: paragraph_with(&doc, "An image follows."),
    };
    doc.0
        .apply_all_with_media(&[insert(p, &media, None)], &[&png()])
        .unwrap();
    for id in [Some(1), Some(0), Some(40), None, Some(2)] {
        edit(&mut doc, insert(p, &media, id));
    }
    let drawings = ids(&doc);
    assert_eq!(drawings.len(), 7);
    assert!(drawings.contains(&Some(1)) && drawings.contains(&Some(40)));
    assert_eq!(
        drawings
            .iter()
            .copied()
            .flatten()
            .collect::<std::collections::HashSet<_>>()
            .len(),
        7
    );
    assert!(doc.0.undo());
    assert_eq!(ids(&doc).len(), 6);
    assert!(doc.0.redo());
    assert_eq!(ids(&doc), drawings);
    assert_eq!(
        ids(&Document::open(&doc.export().unwrap()).unwrap()),
        drawings
    );
    assert_valid("image", &doc.export().unwrap());
}

#[test]
fn drawing_ids_in_preserved_content_are_taken() {
    let base = with_body(&format!(
        r#"<w:p><w:r><w:t>Before</w:t><x:shape xmlns:x="urn:example:shape"><wp:docPr xmlns:wp="{WP}" id="7" name="Shape 7"/></x:shape></w:r></w:p>"#
    ));
    let mut doc = EditingDocument::open(&base).unwrap();
    let media = doc.0.media_name(&png()).unwrap();
    let text = text_node(&doc, "Before");
    doc.0
        .apply_all_with_media(&[insert(at(text, 0), &media, Some(7))], &[&png()])
        .unwrap();
    assert_eq!(ids(&doc), [Some(8)]);
}

#[test]
fn media_a_reopened_document_holds_is_not_added_again() {
    let mut doc = EditingDocument::open(&generated("simple-paragraphs")).unwrap();
    let media = doc.0.media_name(&png()).unwrap();
    let text = text_node(&doc, "First paragraph.");
    doc.0
        .apply_all_with_media(&[insert(at(text, 0), &media, None)], &[&png()])
        .unwrap();
    let saved = doc.export().unwrap();

    let mut doc = EditingDocument::open(&saved).unwrap();
    assert_eq!(doc.0.media_name(&png()).unwrap(), media);
    assert!(doc.dirty_parts().is_empty());
    let text = text_node(&doc, "Second paragraph.");
    edit(&mut doc, insert(at(text, 0), &media, None));
    assert_eq!(ids(&doc), [Some(1), Some(2)]);
    let out = doc.export().unwrap();
    assert_valid("simple-paragraphs", &out);
    assert_eq!(doc.dirty_parts(), [DOCUMENT]);
    assert_eq!(part(&out, RELS), part(&saved, RELS));

    // The name holds other bytes in this package.
    let other = rewrite(&saved, &media, Some(b"\x89PNG\r\n\x1a\nother"));
    let doc = EditingDocument::open(&other).unwrap();
    assert!(
        matches!(
            doc.0.media_name(&png()),
            Err(SessionError::Edit(EditError::Media { .. }))
        ),
        "{:?}",
        doc.0.media_name(&png())
    );
}

#[test]
fn what_is_not_a_picture_is_refused() {
    let mut doc = EditingDocument::open(&generated("simple-paragraphs")).unwrap();
    for bytes in [&b"not a picture"[..], b"", b"\x89PNG", b"GIF88a"] {
        let refused = doc.0.media_name(bytes);
        assert!(
            matches!(refused, Err(SessionError::Edit(EditError::Media { .. }))),
            "{bytes:?}: {refused:?}"
        );
    }
    let media = doc.0.media_name(&png()).unwrap();
    let text = text_node(&doc, "First paragraph.");
    let refused = |doc: &mut EditingDocument, command: Command| {
        let before = stories(doc);
        let result = doc.0.apply_all_with_media(&[command], &[&png()]);
        assert_eq!(stories(doc), before);
        result.unwrap_err()
    };
    for missing in ["word/media/missing.png", "word/styles.xml"] {
        let e = doc.run(insert(at(text, 0), missing, None)).unwrap_err();
        assert!(
            matches!(e, SessionError::Edit(EditError::Media { .. })),
            "{missing}: {e:?}"
        );
    }
    let sized = |width_emu, height_emu| Command::InsertPicture {
        to: None,
        at: at(text, 0),
        media: media.clone(),
        width_emu,
        height_emu,
        description: None,
        id: None,
    };
    for (w, h) in [(0, 1), (1, 0), (27_273_042_316_901, 1)] {
        let e = refused(&mut doc, sized(w, h));
        assert!(
            matches!(e, SessionError::Edit(EditError::Media { .. })),
            "{w}x{h}: {e:?}"
        );
    }
    doc.0
        .apply_all_with_media(&[sized(27_273_042_316_900, 1)], &[&png()])
        .unwrap();
    let text = text_node(&doc, "First paragraph.");
    let e = refused(
        &mut doc,
        Command::InsertPicture {
            to: None,
            at: at(text, 0),
            media,
            width_emu: 1,
            height_emu: 1,
            description: Some("bell \u{7}".into()),
            id: None,
        },
    );
    assert!(
        matches!(e, SessionError::Edit(EditError::InvalidText { .. })),
        "{e:?}"
    );
}

#[test]
fn a_picture_is_refused_when_its_relationship_id_names_something_else() {
    let base = generated("image");
    // Another image than the one the file holds.
    let mut other = png();
    other.extend_from_slice(b"another");
    let media = EditingDocument::open(&base)
        .unwrap()
        .0
        .media_name(&other)
        .unwrap();
    let rel = format!("rIdP{:016x}", fnv(media.as_bytes()));
    let taken = |target: &str| {
        let rels = part(&base, RELS).replace(
            "</Relationships>",
            &format!(
                r#"<Relationship Id="{rel}" Type="{IMAGE}" Target="{target}"/></Relationships>"#
            ),
        );
        rewrite(&base, RELS, Some(rels.as_bytes()))
    };

    // The ID already names the file's own image.
    let bytes = taken("media/image1.png");
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let text = first_editable_text(&doc).unwrap();
    let err = doc
        .0
        .apply_all_with_media(&[insert(at(text, 0), &media, None)], &[&other])
        .unwrap_err();
    assert!(
        matches!(&err, SessionError::Edit(EditError::Media { reason, .. }) if reason.contains("relationship")),
        "{err}"
    );
    assert!(doc.dirty_parts().is_empty());
    assert!(!doc.0.undo());

    // The ID already names this image, which a reopened export does.
    let bytes = with_entry(
        &taken(&format!("media/{}", media.rsplit('/').next().unwrap())),
        &media,
        &other,
    );
    let mut doc = EditingDocument::open(&bytes).unwrap();
    let text = first_editable_text(&doc).unwrap();
    doc.0
        .apply_all_with_media(&[insert(at(text, 0), &media, None)], &[&other])
        .unwrap();
    let out = doc.export().unwrap();
    assert_valid("matching relationship", &out);
    assert_eq!(part(&out, RELS), part(&bytes, RELS));
    let reopened = Document::open(&out).unwrap();
    let shown: Vec<_> = drawings(&reopened)
        .iter()
        .map(|d| d.media[0].part.clone().unwrap())
        .collect();
    assert!(
        shown.contains(&media) && shown.contains(&"word/media/image1.png".to_string()),
        "{shown:?}"
    );
}
