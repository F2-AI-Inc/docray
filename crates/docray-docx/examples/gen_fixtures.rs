//! Generates the committed DOCX corpus. Run from the workspace root:
//! cargo run -p docray-docx --example gen_fixtures
use std::fs::{self, File};
use std::io::Write;
use std::path::Path;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, ZipWriter};

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const MC: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";
const WPS: &str = "http://schemas.microsoft.com/office/word/2010/wordprocessingShape";

const STYLES: &str = r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:asciiTheme="minorHAnsi"/><w:sz w:val="22"/><w:color w:themeColor="accent1"/></w:rPr></w:rPrDefault></w:docDefaults><w:style w:type="paragraph" w:styleId="BoldBase"><w:name w:val="Bold Base"/><w:rPr><w:b/><w:rFonts w:asciiTheme="majorHAnsi"/></w:rPr></w:style><w:style w:type="paragraph" w:styleId="Child"><w:name w:val="Child"/><w:basedOn w:val="BoldBase"/><w:rPr><w:i/><w:sz w:val="28"/></w:rPr></w:style><w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="Heading 1"/><w:pPr><w:outlineLvl w:val="0"/></w:pPr></w:style><w:style w:type="paragraph" w:styleId="Heading2"><w:name w:val="Heading 2"/><w:pPr><w:outlineLvl w:val="1"/></w:pPr></w:style><w:style w:type="paragraph" w:styleId="Title"><w:name w:val="Title"/></w:style><w:style w:type="paragraph" w:styleId="Quote"><w:name w:val="Intense Quote"/></w:style><w:style w:type="paragraph" w:styleId="PageBreak"><w:name w:val="Page Break"/><w:pPr><w:pageBreakBefore/></w:pPr></w:style></w:styles>"#;

const NUMBERING: &str = r#"<w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:abstractNum w:abstractNumId="1"><w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="decimal"/><w:lvlText w:val="%1."/><w:rPr><w:i/></w:rPr></w:lvl><w:lvl w:ilvl="1"><w:start w:val="1"/><w:numFmt w:val="lowerLetter"/><w:lvlText w:val="%1.%2)"/></w:lvl><w:lvl w:ilvl="2"><w:start w:val="1"/><w:numFmt w:val="upperRoman"/><w:lvlText w:val="%1.%2.%3."/><w:lvlRestart w:val="0"/></w:lvl></w:abstractNum><w:abstractNum w:abstractNumId="2"><w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="bullet"/><w:lvlText w:val="x"/><w:lvlPicBulletId w:val="7"/></w:lvl></w:abstractNum><w:num w:numId="10"><w:abstractNumId w:val="1"/></w:num><w:num w:numId="11"><w:abstractNumId w:val="1"/><w:lvlOverride w:ilvl="0"><w:startOverride w:val="3"/></w:lvlOverride></w:num><w:num w:numId="12"><w:abstractNumId w:val="2"/></w:num></w:numbering>"#;

const THEME: &str = r#"<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:themeElements><a:clrScheme name="Fixture"><a:dk1><a:srgbClr val="000000"/></a:dk1><a:accent1><a:srgbClr val="336699"/></a:accent1></a:clrScheme><a:fontScheme name="Fixture"><a:majorFont><a:latin typeface="Fixture Serif"/><a:cs typeface="Fixture Arabic"/></a:majorFont><a:minorFont><a:latin typeface="Fixture Sans"/><a:cs typeface="Fixture Arabic Sans"/></a:minorFont></a:fontScheme></a:themeElements></a:theme>"#;

const CORE: &str = r#"<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>DOCX fixture</dc:title><dc:creator>Fixture Author</dc:creator></cp:coreProperties>"#;

fn document(body: &str) -> String {
    format!(
        r#"<?xml version="1.0"?><w:document xmlns:w="{W}" xmlns:r="{R}" xmlns:mc="{MC}" xmlns:wps="{WPS}" xmlns:v="urn:schemas-microsoft-com:vml"><w:body>{body}</w:body></w:document>"#
    )
}

fn p(text: &str) -> String {
    format!(r#"<w:p><w:r><w:t>{text}</w:t></w:r></w:p>"#)
}

fn sect(extra: &str) -> String {
    format!(
        r#"<w:sectPr>{extra}<w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440"/><w:cols w:num="1"/></w:sectPr>"#
    )
}

fn rels(extra: &str) -> String {
    format!(
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rStyles" Type="{R}/styles" Target="styles.xml"/><Relationship Id="rNumbering" Type="{R}/numbering" Target="numbering.xml"/><Relationship Id="rTheme" Type="{R}/theme" Target="theme/theme1.xml"/>{extra}</Relationships>"#
    )
}

fn content_types(docm: bool, extra: &str) -> String {
    let main = if docm {
        "application/vnd.ms-word.document.macroEnabled.main+xml"
    } else {
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"
    };
    format!(
        r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Default Extension="png" ContentType="image/png"/><Override PartName="/word/document.xml" ContentType="{main}"/>{extra}</Types>"#
    )
}

fn base_entries(body: &str, docm: bool, extra_rels: &str) -> Vec<(String, Vec<u8>)> {
    vec![
        (
            "[Content_Types].xml".into(),
            content_types(docm, "").into_bytes(),
        ),
        ("word/document.xml".into(), document(body).into_bytes()),
        (
            "word/_rels/document.xml.rels".into(),
            rels(extra_rels).into_bytes(),
        ),
        ("word/styles.xml".into(), STYLES.as_bytes().to_vec()),
        ("word/numbering.xml".into(), NUMBERING.as_bytes().to_vec()),
        ("word/theme/theme1.xml".into(), THEME.as_bytes().to_vec()),
        ("docProps/core.xml".into(), CORE.as_bytes().to_vec()),
    ]
}

fn write_zip(path: impl AsRef<Path>, entries: &[(String, Vec<u8>)]) {
    let file = File::create(path).unwrap();
    let mut zip = ZipWriter::new(file);
    let timestamp = DateTime::from_date_and_time(1980, 1, 1, 0, 0, 0).unwrap();
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .last_modified_time(timestamp)
        .unix_permissions(0o644);
    for (name, bytes) in entries {
        zip.start_file(name, options).unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap();
}

fn write_docx(name: &str, body: &str, docm: bool, rels: &str, extras: Vec<(String, Vec<u8>)>) {
    let mut entries = base_entries(body, docm, rels);
    entries.extend(extras);
    let extension = if docm { "docm" } else { "docx" };
    write_zip(format!("testdata/docx/{name}.{extension}"), &entries);
}

fn main() {
    fs::create_dir_all("testdata/docx").unwrap();
    fs::create_dir_all("testdata/malformed").unwrap();

    write_docx(
        "styles",
        &format!(
            r#"<w:p><w:pPr><w:pStyle w:val="Child"/></w:pPr><w:r><w:rPr><w:b/></w:rPr><w:t>Toggle off</w:t></w:r><w:r><w:t> inherited</w:t></w:r></w:p>{}"#,
            sect("")
        ),
        false,
        "",
        vec![],
    );
    write_docx(
        "numbering",
        &format!(
            r#"<w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="10"/></w:numPr></w:pPr><w:r><w:t>one</w:t></w:r></w:p><w:p><w:pPr><w:numPr><w:ilvl w:val="1"/><w:numId w:val="10"/></w:numPr></w:pPr><w:r><w:t>letter</w:t></w:r></w:p><w:p><w:pPr><w:numPr><w:ilvl w:val="2"/><w:numId w:val="10"/></w:numPr></w:pPr><w:r><w:t>roman</w:t></w:r></w:p><w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="11"/></w:numPr></w:pPr><w:r><w:t>restart three</w:t></w:r></w:p><w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="12"/></w:numPr></w:pPr><w:r><w:t>picture bullet</w:t></w:r></w:p>{}"#,
            sect("")
        ),
        false,
        "",
        vec![],
    );
    write_docx(
        "roles",
        &format!(
            r#"<w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>H1</w:t></w:r></w:p><w:p><w:pPr><w:pStyle w:val="Heading2"/></w:pPr><w:r><w:t>H2</w:t></w:r></w:p><w:p><w:pPr><w:pStyle w:val="Title"/></w:pPr><w:r><w:t>Title</w:t></w:r></w:p><w:p><w:pPr><w:pStyle w:val="Quote"/></w:pPr><w:r><w:t>Quote</w:t></w:r></w:p>{}"#,
            sect("")
        ),
        false,
        "",
        vec![],
    );
    write_docx(
        "fields",
        &format!(
            r#"<w:p><w:fldSimple w:instr="PAGE"><w:r><w:t>3</w:t></w:r></w:fldSimple></w:p><w:p><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText> TOC \o &quot;1-3&quot; </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>Cached heading</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r></w:p>{}"#,
            sect("")
        ),
        false,
        "",
        vec![],
    );
    write_docx(
        "mc",
        &format!(
            r#"<w:p><w:r><mc:AlternateContent><mc:Choice Requires="wps"><w:drawing><wp:anchor xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"><wp:positionH relativeFrom="margin"><wp:posOffset>127000</wp:posOffset></wp:positionH><wp:positionV relativeFrom="paragraph"><wp:posOffset>254000</wp:posOffset></wp:positionV><wp:extent cx="1270000" cy="635000"/><wps:txbx><w:txbxContent>{}</w:txbxContent></wps:txbx></wp:anchor></w:drawing></mc:Choice><mc:Fallback><w:pict><v:shape><v:textbox><w:txbxContent>{}</w:txbxContent></v:textbox></v:shape></w:pict></mc:Fallback></mc:AlternateContent></w:r></w:p><w:p><w:r><mc:AlternateContent><mc:Choice Requires="unsupported">{}</mc:Choice><mc:Fallback><w:pict><v:shape><v:textbox><w:txbxContent>{}</w:txbxContent></v:textbox></v:shape></w:pict></mc:Fallback></mc:AlternateContent></w:r></w:p>{}"#,
            p("Choice text"),
            p("Unselected fallback"),
            p("Unselected choice"),
            p("VML fallback selected"),
            sect("")
        ),
        false,
        "",
        vec![],
    );
    write_docx(
        "tables",
        &format!(
            r#"<w:tbl><w:tblPr><w:tblpPr w:horzAnchor="margin" w:tblpX="720" w:tblpY="360"/></w:tblPr><w:tblGrid><w:gridCol w:w="1440"/><w:gridCol w:w="2880"/></w:tblGrid><w:tr><w:tc><w:tcPr><w:gridSpan w:val="2"/><w:vMerge w:val="restart"/></w:tcPr>{}</w:tc></w:tr><w:tr><w:tc><w:tcPr><w:gridSpan w:val="2"/><w:vMerge/></w:tcPr>{}</w:tc></w:tr><w:tr><w:tc>{}<w:tbl><w:tblGrid><w:gridCol w:w="720"/></w:tblGrid><w:tr><w:tc>{}</w:tc></w:tr></w:tbl></w:tc><w:tc>{}</w:tc></w:tr></w:tbl>{}"#,
            p("merged"),
            p("skip"),
            p("outer"),
            p("nested"),
            p("right"),
            sect("")
        ),
        false,
        "",
        vec![],
    );
    let image_rels = format!(
        r#"<Relationship Id="rImg" Type="{R}/image" Target="media/image1.png"/><Relationship Id="rExternal" Type="{R}/image" Target="https://example.test/image.png" TargetMode="External"/>"#
    );
    write_docx(
        "images",
        &format!(
            r#"<w:p><w:r><w:drawing><wp:inline xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"><wp:extent cx="1270000" cy="635000"/><wp:docPr id="1" name="Inline" descr="Inline alt"/><a:graphic xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:graphicData><a:blip r:embed="rImg"/></a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p><w:p><w:r><w:drawing><wp:anchor xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"><wp:positionH relativeFrom="page"><wp:posOffset>254000</wp:posOffset></wp:positionH><wp:positionV relativeFrom="margin"><wp:align>center</wp:align></wp:positionV><wp:extent cx="2540000" cy="1270000"/><wp:docPr id="2" name="Anchor" descr="Anchor alt"/><a:graphic xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:graphicData><a:blip r:embed="rImg"/></a:graphicData></a:graphic></wp:anchor></w:drawing></w:r></w:p><w:p><w:r><w:drawing><wp:inline xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"><wp:extent cx="635000" cy="635000"/><wp:docPr id="3" name="Missing" descr="Missing alt"/><a:graphic xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:graphicData><a:blip r:embed="rMissing"/></a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p><w:p><w:r><w:drawing><wp:inline xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"><wp:extent cx="635000" cy="635000"/><wp:docPr id="4" name="External"/><a:graphic xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:graphicData><a:blip r:embed="rExternal"/></a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p><w:p><w:r><w:t>Office art policy</w:t><w:pict><v:rect style="width:72pt;height:1pt"/></w:pict><w:drawing/></w:r></w:p>{}"#,
            sect("")
        ),
        false,
        &image_rels,
        vec![(
            "word/media/image1.png".into(),
            b"deterministic-image".to_vec(),
        )],
    );
    let hyperlink_rels = format!(
        r#"<Relationship Id="rLink" Type="{R}/hyperlink" Target="https://example.test/literal?x=1&amp;y=2" TargetMode="External"/>"#
    );
    write_docx(
        "hyperlinks",
        &format!(
            r#"<w:p><w:hyperlink r:id="rLink"><w:r><w:t>external</w:t></w:r></w:hyperlink><w:r><w:t> </w:t></w:r><w:hyperlink w:anchor="bookmark"><w:r><w:t>internal</w:t></w:r></w:hyperlink></w:p>{}"#,
            sect("")
        ),
        false,
        &hyperlink_rels,
        vec![],
    );
    let comments = format!(
        r#"<w:comments xmlns:w="{W}"><w:comment w:id="7" w:author="PRIVATE" w:date="2020-01-01T00:00:00Z">{}</w:comment></w:comments>"#,
        p("Review this")
    );
    write_docx(
        "revisions-comments",
        &format!(
            r#"<w:p><w:commentRangeStart w:id="7"/><w:ins><w:r><w:t>inserted</w:t></w:r></w:ins><w:del><w:r><w:delText>deleted</w:delText></w:r></w:del><w:r><w:commentReference w:id="7"/></w:r></w:p>{}"#,
            sect("")
        ),
        false,
        &format!(r#"<Relationship Id="rComments" Type="{R}/comments" Target="comments.xml"/>"#),
        vec![("word/comments.xml".into(), comments.into_bytes())],
    );
    let header = format!(r#"<w:hdr xmlns:w="{W}">{}</w:hdr>"#, p("Header default"));
    let header_first = format!(r#"<w:hdr xmlns:w="{W}">{}</w:hdr>"#, p("Header first"));
    let footer = format!(r#"<w:ftr xmlns:w="{W}">{}</w:ftr>"#, p("Footer even"));
    let footnotes = format!(
        r#"<w:footnotes xmlns:w="{W}"><w:footnote w:id="2">{}</w:footnote></w:footnotes>"#,
        p("Footnote body")
    );
    let endnotes = format!(
        r#"<w:endnotes xmlns:w="{W}"><w:endnote w:id="3">{}</w:endnote></w:endnotes>"#,
        p("Endnote body")
    );
    let story_rels = format!(
        r#"<Relationship Id="rHeader" Type="{R}/header" Target="header1.xml"/><Relationship Id="rHeaderFirst" Type="{R}/header" Target="header2.xml"/><Relationship Id="rFooter" Type="{R}/footer" Target="footer1.xml"/><Relationship Id="rFootnotes" Type="{R}/footnotes" Target="footnotes.xml"/><Relationship Id="rEndnotes" Type="{R}/endnotes" Target="endnotes.xml"/>"#
    );
    write_docx(
        "stories",
        &format!(
            r#"<w:p><w:r><w:t>Body with notes</w:t></w:r><w:r><w:footnoteReference w:id="2"/><w:endnoteReference w:id="3"/></w:r></w:p>{}"#,
            sect(
                r#"<w:headerReference w:type="default" r:id="rHeader"/><w:headerReference w:type="first" r:id="rHeaderFirst"/><w:footerReference w:type="even" r:id="rFooter"/>"#
            )
        ),
        false,
        &story_rels,
        vec![
            ("word/header1.xml".into(), header.into_bytes()),
            ("word/header2.xml".into(), header_first.into_bytes()),
            ("word/footer1.xml".into(), footer.into_bytes()),
            ("word/footnotes.xml".into(), footnotes.into_bytes()),
            ("word/endnotes.xml".into(), endnotes.into_bytes()),
        ],
    );
    write_docx(
        "sections",
        &format!(
            r#"<w:p><w:pPr>{}</w:pPr><w:r><w:t>Section one</w:t></w:r></w:p>{}{}"#,
            sect(r#"<w:pgSz w:w="11906" w:h="16838"/><w:cols w:num="2"/>"#),
            p("Section two"),
            sect(r#"<w:pgSz w:w="12240" w:h="15840"/>"#)
        ),
        false,
        "",
        vec![],
    );
    write_docx(
        "breaks",
        &format!(
            r#"<w:p><w:r><w:t>before</w:t><w:br w:type="page"/><w:t>after page</w:t><w:lastRenderedPageBreak/><w:t>after hint</w:t></w:r></w:p><w:p><w:pPr><w:pStyle w:val="PageBreak"/></w:pPr><w:r><w:t>style break</w:t><w:br w:type="column"/><w:lastRenderedPageBreak/></w:r></w:p>{}"#,
            sect("")
        ),
        false,
        "",
        vec![],
    );
    write_docx(
        "wrappers",
        &format!(
            r#"<w:sdt><w:sdtPr><w:alias w:val="Block control"/><w:text/></w:sdtPr><w:sdtContent>{}</w:sdtContent></w:sdt><w:smartTag>{}</w:smartTag><w:customXml>{}</w:customXml><w:p><w:sdt><w:sdtPr><w:text/></w:sdtPr><w:sdtContent><w:r><w:t>inline sdt</w:t></w:r></w:sdtContent></w:sdt><w:r><w:t> + </w:t></w:r><w:smartTag><w:r><w:t>smart tag</w:t></w:r></w:smartTag><w:r><w:t> + </w:t></w:r><w:customXml><w:r><w:t>custom XML</w:t></w:r></w:customXml></w:p><w:sdt><w:sdtPr/><w:sdtContent><w:tbl><w:tblGrid><w:gridCol w:w="1440"/></w:tblGrid><w:tr><w:tc><w:sdt><w:sdtPr/><w:sdtContent>{}</w:sdtContent></w:sdt></w:tc></w:tr></w:tbl></w:sdtContent></w:sdt>{}"#,
            p("block sdt"),
            p("block smart tag"),
            p("block custom XML"),
            p("cell sdt"),
            sect("")
        ),
        false,
        "",
        vec![],
    );
    write_docx(
        "body-pagination",
        &format!(
            r#"{}<w:tbl><w:tblGrid><w:gridCol w:w="2880"/></w:tblGrid><w:tr><w:tc><w:p><w:r><w:t>table page one</w:t><w:lastRenderedPageBreak/><w:t>table page two</w:t></w:r></w:p><w:tbl><w:tblGrid><w:gridCol w:w="1440"/></w:tblGrid><w:tr><w:tc><w:p><w:r><w:t>nested page two</w:t><w:lastRenderedPageBreak/><w:t>nested page three</w:t></w:r></w:p></w:tc></w:tr></w:tbl><w:p><w:r><w:drawing><wp:inline xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"><wp:extent cx="1270000" cy="635000"/><wps:txbx><w:txbxContent><w:p><w:r><w:t>textbox page three</w:t><w:lastRenderedPageBreak/><w:t>textbox page four</w:t></w:r></w:p></w:txbxContent></wps:txbx></wp:inline></w:drawing></w:r></w:p></w:tc></w:tr></w:tbl>{}{}"#,
            p("outside page one"),
            p("outside page four"),
            sect("")
        ),
        false,
        "",
        vec![],
    );
    write_docx(
        "zero-hints",
        &format!("{}{}", p("No pagination cache"), sect("")),
        false,
        "",
        vec![],
    );
    write_docx(
        "rtl",
        &format!(
            r#"<w:p><w:r><w:rPr><w:rtl/><w:rFonts w:cstheme="majorBidi"/><w:szCs w:val="30"/><w:bCs/></w:rPr><w:t>שלום logical</w:t></w:r></w:p>{}"#,
            sect("")
        ),
        false,
        "",
        vec![],
    );
    write_docx(
        "macro",
        &format!("{}{}", p("Macro document text"), sect("")),
        true,
        "",
        vec![("word/vbaProject.bin".into(), b"DO NOT READ".to_vec())],
    );
    write_docx(
        "run-merge",
        &format!(
            r#"<w:p><w:r><w:t>A</w:t></w:r><w:r><w:t>B</w:t></w:r><w:r><w:t>C</w:t></w:r><w:r><w:rPr><w:b/></w:rPr><w:t>D</w:t></w:r></w:p>{}"#,
            sect("")
        ),
        false,
        "",
        vec![],
    );

    let nested_begin = "<w:r><w:fldChar w:fldCharType=\"begin\"/></w:r>".repeat(20);
    let nested_end = "<w:r><w:fldChar w:fldCharType=\"end\"/></w:r>".repeat(20);
    write_docx(
        "../malformed/docx-field-nesting",
        &format!(
            "<w:p>{nested_begin}<w:r><w:instrText>SECRET</w:instrText></w:r>{nested_end}</w:p>{}",
            sect("")
        ),
        false,
        "",
        vec![],
    );

    let cycle_styles = format!(
        r#"<w:styles xmlns:w="{W}"><w:style w:type="paragraph" w:styleId="A"><w:basedOn w:val="B"/></w:style><w:style w:type="paragraph" w:styleId="B"><w:basedOn w:val="A"/></w:style></w:styles>"#
    );
    let mut cycle = base_entries(
        &format!(
            r#"<w:p><w:pPr><w:pStyle w:val="A"/></w:pPr><w:r><w:t>cycle</w:t></w:r></w:p>{}"#,
            sect("")
        ),
        false,
        "",
    );
    cycle.retain(|(name, _)| name != "word/styles.xml");
    cycle.push(("word/styles.xml".into(), cycle_styles.into_bytes()));
    write_zip("testdata/malformed/docx-style-cycle.docx", &cycle);

    let mut mc_bomb = p("bottom");
    for _ in 0..40 {
        mc_bomb = format!(
            r#"<mc:AlternateContent><mc:Choice Requires="unsupported"><w:p><w:r><w:t>bad</w:t></w:r></w:p></mc:Choice><mc:Fallback>{mc_bomb}</mc:Fallback></mc:AlternateContent>"#
        );
    }
    write_docx(
        "../malformed/docx-mc-bomb",
        &format!("{mc_bomb}{}", sect("")),
        false,
        "",
        vec![],
    );

    let broken_numbering = format!(
        r#"<w:numbering xmlns:w="{W}"><w:num w:numId="99"><w:abstractNumId w:val="99"/></w:num></w:numbering>"#
    );
    let mut number_cycle = base_entries(
        &format!(
            r#"<w:p><w:pPr><w:numPr><w:numId w:val="99"/></w:numPr></w:pPr><w:r><w:t>broken numbering</w:t></w:r></w:p>{}"#,
            sect("")
        ),
        false,
        "",
    );
    number_cycle.retain(|(name, _)| name != "word/numbering.xml");
    number_cycle.push(("word/numbering.xml".into(), broken_numbering.into_bytes()));
    write_zip(
        "testdata/malformed/docx-numbering-cycle.docx",
        &number_cycle,
    );

    let mut huge = String::new();
    for index in 0..250_001 {
        huge.push_str(&format!("<w:p/><!--{index}-->"));
    }
    huge.push_str(&sect(""));
    write_docx("../malformed/docx-huge-story", &huge, false, "", vec![]);

    println!("DOCX fixtures written to testdata/docx and testdata/malformed");
}
