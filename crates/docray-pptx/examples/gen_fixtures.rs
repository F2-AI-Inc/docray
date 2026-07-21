//! Generates the committed PPTX corpus. Run from the workspace root:
//! cargo run -p docray-pptx --example gen_fixtures
use std::fs::{self, File};
use std::io::Write;
use std::path::Path;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, ZipWriter};

const PRESENTATION: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:presentation xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><p:sldIdLst><p:sldId id="256" r:id="rId1"/></p:sldIdLst><p:sldSz cx="9144000" cy="6858000"/></p:presentation>"#;

const PRESENTATION_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide1.xml"/></Relationships>"#;

const ROOT_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="ppt/presentation.xml"/></Relationships>"#;

const CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Default Extension="bin" ContentType="application/octet-stream"/><Override PartName="/ppt/presentation.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"/><Override PartName="/ppt/slides/slide1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slide+xml"/><Override PartName="/ppt/slideLayouts/slideLayout1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml"/><Override PartName="/ppt/slideMasters/slideMaster1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slideMaster+xml"/><Override PartName="/ppt/theme/theme1.xml" ContentType="application/vnd.openxmlformats-officedocument.theme+xml"/></Types>"#;

const THEME: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" name="Fixture"><a:themeElements><a:clrScheme name="Fixture"><a:dk1><a:srgbClr val="102030"/></a:dk1><a:lt1><a:srgbClr val="FFFFFF"/></a:lt1><a:accent1><a:srgbClr val="336699"/></a:accent1></a:clrScheme><a:fontScheme name="Fixture"><a:majorFont><a:latin typeface="Fixture Serif"/></a:majorFont><a:minorFont><a:latin typeface="Fixture Sans"/></a:minorFont></a:fontScheme></a:themeElements></a:theme>"#;

const LAYOUT: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<p:sldLayout xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><p:cSld><p:spTree><p:nvGrpSpPr/><p:grpSpPr/><p:sp><p:nvSpPr><p:cNvPr id="2" name="Layout title"/><p:cNvSpPr/><p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr><p:spPr><a:xfrm><a:off x="914400" y="457200"/><a:ext cx="7315200" cy="914400"/></a:xfrm><a:prstGeom prst="rect"/></p:spPr><p:txBody><a:bodyPr/><a:lstStyle><a:lvl1pPr><a:defRPr sz="3200"><a:latin typeface="+mj-lt"/></a:defRPr></a:lvl1pPr></a:lstStyle><a:p/></p:txBody></p:sp><p:sp><p:nvSpPr><p:cNvPr id="3" name="Layout body"/><p:cNvSpPr/><p:nvPr><p:ph type="body" idx="1"/></p:nvPr></p:nvSpPr><p:spPr/><p:txBody><a:bodyPr/><a:lstStyle/><a:p/></p:txBody></p:sp></p:spTree></p:cSld></p:sldLayout>"#;

const LAYOUT_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster" Target="../slideMasters/slideMaster1.xml"/></Relationships>"#;

const MASTER: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<p:sldMaster xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><p:cSld><p:spTree><p:nvGrpSpPr/><p:grpSpPr/><p:sp><p:nvSpPr><p:cNvPr id="2" name="Master body"/><p:cNvSpPr/><p:nvPr><p:ph type="body" idx="1"/></p:nvPr></p:nvSpPr><p:spPr><a:xfrm><a:off x="914400" y="1828800"/><a:ext cx="7315200" cy="3657600"/></a:xfrm><a:prstGeom prst="rect"/></p:spPr><p:txBody><a:bodyPr/><a:lstStyle/><a:p/></p:txBody></p:sp></p:spTree></p:cSld><p:clrMap accent1="accent1" tx1="dk1" bg1="lt1"/><p:txStyles><p:titleStyle><a:lvl1pPr><a:defRPr sz="3000"><a:latin typeface="+mj-lt"/></a:defRPr></a:lvl1pPr></p:titleStyle><p:bodyStyle><a:lvl1pPr><a:defRPr sz="2000"><a:latin typeface="+mn-lt"/></a:defRPr></a:lvl1pPr></p:bodyStyle><p:otherStyle><a:lvl1pPr><a:defRPr sz="1800"><a:latin typeface="+mn-lt"/></a:defRPr></a:lvl1pPr></p:otherStyle></p:txStyles></p:sldMaster>"#;

const MASTER_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme" Target="../theme/theme1.xml"/></Relationships>"#;

fn slide(body: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><p:cSld><p:spTree><p:nvGrpSpPr/><p:grpSpPr/>{body}</p:spTree></p:cSld></p:sld>"#
    )
}

fn hidden_slide(body: &str) -> String {
    slide(body).replacen("<p:sld ", "<p:sld show=\"0\" ", 1)
}

fn notes_slide() -> String {
    r#"<?xml version="1.0" encoding="UTF-8"?>
<p:notes xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><p:cSld><p:spTree><p:nvGrpSpPr/><p:grpSpPr/><p:sp><p:nvSpPr><p:cNvPr id="2" name="Slide image"/><p:cNvSpPr/><p:nvPr><p:ph type="sldImg"/></p:nvPr></p:nvSpPr><p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>IGNORE SLIDE IMAGE</a:t></a:r></a:p></p:txBody></p:sp><p:sp><p:nvSpPr><p:cNvPr id="3" name="Notes body"/><p:cNvSpPr/><p:nvPr><p:ph type="body"/></p:nvPr></p:nvSpPr><p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>Presenter script line one</a:t></a:r></a:p><a:p><a:r><a:t>line two</a:t></a:r></a:p></p:txBody></p:sp><p:sp><p:nvSpPr><p:cNvPr id="4" name="Slide number"/><p:cNvSpPr/><p:nvPr><p:ph type="sldNum"/></p:nvPr></p:nvSpPr><p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>IGNORE SLIDE NUMBER</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:notes>"#
        .to_string()
}

fn shape(id: u32, name: &str, x: i64, y: i64, cx: i64, cy: i64, text: &str) -> String {
    format!(
        r#"<p:sp><p:nvSpPr><p:cNvPr id="{id}" name="{name}"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x="{x}" y="{y}"/><a:ext cx="{cx}" cy="{cy}"/></a:xfrm><a:prstGeom prst="rect"/><a:solidFill><a:srgbClr val="EEDDAA"/></a:solidFill></p:spPr><p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:rPr sz="1800"><a:latin typeface="Fixture Sans"/><a:solidFill><a:srgbClr val="112233"/></a:solidFill></a:rPr><a:t>{text}</a:t></a:r></a:p></p:txBody></p:sp>"#
    )
}

fn default_slide_rels(extra: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rIdLayout" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout" Target="../slideLayouts/slideLayout1.xml"/>{extra}</Relationships>"#
    )
}

fn package_entries(
    slide_xml: String,
    slide_rels: String,
    has_notes: bool,
) -> Vec<(String, Vec<u8>)> {
    let content_types = if has_notes {
        CONTENT_TYPES.replace(
            "</Types>",
            r#"<Override PartName="/ppt/notesSlides/notesSlide1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.notesSlide+xml"/></Types>"#,
        )
    } else {
        CONTENT_TYPES.to_string()
    };
    vec![
        ("[Content_Types].xml".into(), content_types.into_bytes()),
        ("_rels/.rels".into(), ROOT_RELS.as_bytes().to_vec()),
        (
            "ppt/presentation.xml".into(),
            PRESENTATION.as_bytes().to_vec(),
        ),
        (
            "ppt/_rels/presentation.xml.rels".into(),
            PRESENTATION_RELS.as_bytes().to_vec(),
        ),
        ("ppt/slides/slide1.xml".into(), slide_xml.into_bytes()),
        (
            "ppt/slides/_rels/slide1.xml.rels".into(),
            slide_rels.into_bytes(),
        ),
        (
            "ppt/slideLayouts/slideLayout1.xml".into(),
            LAYOUT.as_bytes().to_vec(),
        ),
        (
            "ppt/slideLayouts/_rels/slideLayout1.xml.rels".into(),
            LAYOUT_RELS.as_bytes().to_vec(),
        ),
        (
            "ppt/slideMasters/slideMaster1.xml".into(),
            MASTER.as_bytes().to_vec(),
        ),
        (
            "ppt/slideMasters/_rels/slideMaster1.xml.rels".into(),
            MASTER_RELS.as_bytes().to_vec(),
        ),
        ("ppt/theme/theme1.xml".into(), THEME.as_bytes().to_vec()),
    ]
}

fn write_zip(path: impl AsRef<Path>, entries: &[(String, Vec<u8>)], method: CompressionMethod) {
    let file = File::create(path).unwrap();
    let mut zip = ZipWriter::new(file);
    let timestamp = DateTime::from_date_and_time(1980, 1, 1, 0, 0, 0).unwrap();
    let options = SimpleFileOptions::default()
        .compression_method(method)
        .last_modified_time(timestamp)
        .unix_permissions(0o644);
    for (name, bytes) in entries {
        zip.start_file(name, options).unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap();
}

fn write_pptx(name: &str, slide_xml: String, rels: String, extras: Vec<(String, Vec<u8>)>) {
    let has_notes = extras
        .iter()
        .any(|(path, _)| path.starts_with("ppt/notesSlides/"));
    let mut entries = package_entries(slide_xml, rels, has_notes);
    let mut overrides = String::new();
    for (path, _) in &extras {
        let content_type = if path.starts_with("ppt/charts/") {
            Some("application/vnd.openxmlformats-officedocument.drawingml.chart+xml")
        } else if path.starts_with("ppt/diagrams/data") {
            Some("application/vnd.openxmlformats-officedocument.drawingml.diagramData+xml")
        } else {
            None
        };
        if let Some(content_type) = content_type {
            overrides.push_str(&format!(
                r#"<Override PartName="/{path}" ContentType="{content_type}"/>"#
            ));
        }
    }
    if !overrides.is_empty() {
        let content_types = &mut entries[0].1;
        let updated = String::from_utf8(content_types.clone())
            .unwrap()
            .replace("</Types>", &format!("{overrides}</Types>"));
        *content_types = updated.into_bytes();
    }
    entries.extend(extras);
    write_zip(
        format!("testdata/pptx/{name}.pptx"),
        &entries,
        CompressionMethod::Stored,
    );
}

fn main() {
    fs::create_dir_all("testdata/pptx").unwrap();
    fs::create_dir_all("testdata/malformed").unwrap();

    let basic = slide(&format!(
        "{}{}",
        shape(2, "First", 914400, 914400, 2743200, 914400, "First shape"),
        shape(
            3,
            "Second",
            4572000,
            2286000,
            2743200,
            1371600,
            "Second shape"
        )
    ));
    write_pptx("basic", basic, default_slide_rels(""), vec![]);

    let placeholders = slide(
        r#"<p:sp><p:nvSpPr><p:cNvPr id="2" name="Title"/><p:cNvSpPr/><p:nvPr><p:ph type="ctrTitle"/></p:nvPr></p:nvSpPr><p:spPr/><p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>Inherited title</a:t></a:r></a:p></p:txBody></p:sp><p:sp><p:nvSpPr><p:cNvPr id="3" name="Body"/><p:cNvSpPr/><p:nvPr><p:ph type="body" idx="1"/></p:nvPr></p:nvSpPr><p:spPr/><p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>Inherited body</a:t></a:r></a:p></p:txBody></p:sp>"#,
    );
    write_pptx("placeholders", placeholders, default_slide_rels(""), vec![]);

    let groups = slide(
        r#"<p:grpSp><p:nvGrpSpPr/><p:grpSpPr><a:xfrm><a:off x="127000" y="254000"/><a:ext cx="5080000" cy="2540000"/><a:chOff x="127000" y="127000"/><a:chExt cx="2540000" cy="1270000"/></a:xfrm></p:grpSpPr><p:grpSp><p:nvGrpSpPr/><p:grpSpPr><a:xfrm><a:off x="254000" y="254000"/><a:ext cx="1270000" cy="635000"/><a:chOff x="0" y="0"/><a:chExt cx="1270000" cy="635000"/></a:xfrm></p:grpSpPr><p:sp><p:nvSpPr><p:cNvPr id="4" name="Nested"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm rot="5400000" flipH="1"><a:off x="127000" y="127000"/><a:ext cx="508000" cy="254000"/></a:xfrm><a:prstGeom prst="rect"/></p:spPr><p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:rPr sz="1800"/><a:t>Nested transform</a:t></a:r></a:p></p:txBody></p:sp></p:grpSp></p:grpSp>"#,
    );
    write_pptx("groups", groups, default_slide_rels(""), vec![]);

    let picture = slide(
        r#"<p:pic><p:nvPicPr><p:cNvPr id="2" name="Picture"/><p:cNvPicPr/><p:nvPr/></p:nvPicPr><p:blipFill><a:blip r:embed="rIdImage"/></p:blipFill><p:spPr><a:xfrm rot="5400000"><a:off x="1270000" y="1270000"/><a:ext cx="1016000" cy="508000"/></a:xfrm><a:prstGeom prst="rect"/></p:spPr></p:pic>"#,
    );
    write_pptx(
        "picture",
        picture,
        default_slide_rels(
            r#"<Relationship Id="rIdImage" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="../media/image1.bin"/>"#,
        ),
        vec![(
            "ppt/media/image1.bin".into(),
            b"deterministic fixture image bytes".to_vec(),
        )],
    );

    let hidden_context = hidden_slide(
        r#"<p:sp><p:nvSpPr><p:cNvPr id="2" name="Default body" descr="Shape alternative text"/><p:cNvSpPr/><p:nvPr><p:ph/></p:nvPr></p:nvSpPr><p:spPr><a:xfrm><a:off x="914400" y="914400"/><a:ext cx="3657600" cy="914400"/></a:xfrm><a:prstGeom prst="rect"/></p:spPr><p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>Hidden channel fixture</a:t></a:r></a:p></p:txBody></p:sp><p:pic><p:nvPicPr><p:cNvPr id="3" name="Revenue chart" title="Chart showing Q3 revenue"/><p:cNvPicPr/><p:nvPr/></p:nvPicPr><p:blipFill><a:blip r:embed="rIdImage"/></p:blipFill><p:spPr><a:xfrm><a:off x="914400" y="2286000"/><a:ext cx="2540000" cy="1270000"/></a:xfrm><a:prstGeom prst="rect"/></p:spPr></p:pic>"#,
    );
    write_pptx(
        "hidden-context",
        hidden_context,
        default_slide_rels(
            r#"<Relationship Id="rIdImage" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="../media/hidden-image.bin"/><Relationship Id="rIdNotes" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/notesSlide" Target="../notesSlides/notesSlide1.xml"/>"#,
        ),
        vec![
            (
                "ppt/media/hidden-image.bin".into(),
                b"hidden context fixture image bytes".to_vec(),
            ),
            (
                "ppt/notesSlides/notesSlide1.xml".into(),
                notes_slide().into_bytes(),
            ),
        ],
    );

    let table = slide(
        r#"<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="2" name="Table"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr><p:xfrm><a:off x="914400" y="1143000"/><a:ext cx="2540000" cy="1016000"/></p:xfrm><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/table"><a:tbl><a:tblGrid><a:gridCol w="1016000"/><a:gridCol w="1524000"/></a:tblGrid><a:tr h="381000"><a:tc gridSpan="2"><a:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:rPr sz="1800"/><a:t>Merged</a:t></a:r></a:p></a:txBody><a:tcPr/></a:tc><a:tc hMerge="1"><a:txBody><a:bodyPr/><a:lstStyle/><a:p/></a:txBody><a:tcPr/></a:tc></a:tr><a:tr h="635000"><a:tc><a:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:rPr sz="1800"/><a:t>Left</a:t></a:r></a:p></a:txBody><a:tcPr/></a:tc><a:tc><a:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:rPr sz="1800"/><a:t>Right</a:t></a:r></a:p></a:txBody><a:tcPr/></a:tc></a:tr></a:tbl></a:graphicData></a:graphic></p:graphicFrame>"#,
    );
    write_pptx("table", table, default_slide_rels(""), vec![]);

    let chart = slide(
        r#"<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="2" name="Revenue chart"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr><p:xfrm><a:off x="914400" y="914400"/><a:ext cx="4572000" cy="2743200"/></p:xfrm><a:graphic><a:graphicData xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" uri="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart r:id="rIdChart"/></a:graphicData></a:graphic></p:graphicFrame>"#,
    );
    let chart_part = r#"<?xml version="1.0" encoding="UTF-8"?>
<c:chartSpace xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart><c:title><c:tx><c:rich><a:p><a:r><a:t>Quarterly revenue</a:t></a:r></a:p></c:rich></c:tx></c:title><c:plotArea><c:barChart><c:ser><c:tx><c:rich><a:p><a:r><a:t>Revenue</a:t></a:r></a:p></c:rich></c:tx><c:cat><c:strLit><c:pt idx="0"><c:v>Q1</c:v></c:pt><c:pt idx="1"><c:v>Q2</c:v></c:pt></c:strLit></c:cat><c:val><c:numLit><c:pt idx="0"><c:v>10.5</c:v></c:pt><c:pt idx="1"><c:v>12</c:v></c:pt></c:numLit></c:val></c:ser><c:ser><c:tx><c:strRef><c:strCache><c:pt idx="0"><c:v>Costs</c:v></c:pt></c:strCache></c:strRef></c:tx><c:cat><c:strLit><c:pt idx="0"><c:v>Q1</c:v></c:pt><c:pt idx="1"><c:v>Q2</c:v></c:pt></c:strLit></c:cat><c:val><c:numLit><c:pt idx="0"><c:v>7</c:v></c:pt><c:pt idx="1"><c:v>8.25</c:v></c:pt></c:numLit></c:val></c:ser></c:barChart><c:catAx><c:title><c:tx><c:rich><a:p><a:r><a:t>Quarter</a:t></a:r></a:p></c:rich></c:tx></c:title></c:catAx><c:valAx><c:title><c:tx><c:rich><a:p><a:r><a:t>USD millions</a:t></a:r></a:p></c:rich></c:tx></c:title></c:valAx></c:plotArea></c:chart></c:chartSpace>"#;
    write_pptx(
        "chart",
        chart,
        default_slide_rels(
            r#"<Relationship Id="rIdChart" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart" Target="../charts/chart1.xml"/>"#,
        ),
        vec![(
            "ppt/charts/chart1.xml".into(),
            chart_part.as_bytes().to_vec(),
        )],
    );

    let smartart = slide(
        r#"<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="2" name="Process diagram"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr><p:xfrm><a:off x="1270000" y="1016000"/><a:ext cx="5080000" cy="2540000"/></p:xfrm><a:graphic><a:graphicData xmlns:dgm="http://schemas.openxmlformats.org/drawingml/2006/diagram" uri="http://schemas.openxmlformats.org/drawingml/2006/diagram"><dgm:relIds r:dm="rIdDiagram"/></a:graphicData></a:graphic></p:graphicFrame>"#,
    );
    let diagram_part = r#"<?xml version="1.0" encoding="UTF-8"?>
<dgm:dataModel xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:dgm="http://schemas.openxmlformats.org/drawingml/2006/diagram"><dgm:ptLst><dgm:pt modelId="1"><dgm:t><a:p><a:r><a:t>Discover</a:t></a:r></a:p></dgm:t></dgm:pt><dgm:pt modelId="2"><dgm:t><a:p><a:r><a:t>Build</a:t></a:r></a:p></dgm:t></dgm:pt><dgm:pt modelId="3"><dgm:t><a:p><a:r><a:t>Deliver</a:t></a:r></a:p></dgm:t></dgm:pt></dgm:ptLst></dgm:dataModel>"#;
    write_pptx(
        "smartart",
        smartart,
        default_slide_rels(
            r#"<Relationship Id="rIdDiagram" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/diagramData" Target="../diagrams/data1.xml"/>"#,
        ),
        vec![(
            "ppt/diagrams/data1.xml".into(),
            diagram_part.as_bytes().to_vec(),
        )],
    );

    let graphic_picture = slide(
        r#"<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="2" name="Framed picture"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr><p:xfrm><a:off x="2540000" y="1270000"/><a:ext cx="2032000" cy="1016000"/></p:xfrm><a:graphic><a:graphicData xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture" uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic><pic:nvPicPr><pic:cNvPr id="3" name="Embedded picture" descr="Graphic-frame picture alternative"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed="rIdGraphicImage"/></pic:blipFill></pic:pic></a:graphicData></a:graphic></p:graphicFrame>"#,
    );
    write_pptx(
        "graphic-picture",
        graphic_picture,
        default_slide_rels(
            r#"<Relationship Id="rIdGraphicImage" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="../media/graphic-image.bin"/>"#,
        ),
        vec![(
            "ppt/media/graphic-image.bin".into(),
            b"deterministic graphic frame image bytes".to_vec(),
        )],
    );

    // Covers three graphicFrame warning paths on one slide: a chart whose part
    // is missing, a chart whose part parses but has no extractable text, and a
    // surviving shape proving the rest of the slide still extracts.
    let missing_chart = slide(&format!(
        "{}{}{}",
        r#"<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="2" name="Missing chart"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr><p:xfrm><a:off x="914400" y="914400"/><a:ext cx="3657600" cy="1828800"/></p:xfrm><a:graphic><a:graphicData xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" uri="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart r:id="rIdMissingChart"/></a:graphicData></a:graphic></p:graphicFrame>"#,
        r#"<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="4" name="Empty chart"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr><p:xfrm><a:off x="4572000" y="914400"/><a:ext cx="3657600" cy="1828800"/></p:xfrm><a:graphic><a:graphicData xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" uri="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart r:id="rIdEmptyChart"/></a:graphicData></a:graphic></p:graphicFrame>"#,
        shape(
            3,
            "Survivor",
            914400,
            3657600,
            3657600,
            914400,
            "Slide still extracts"
        )
    ));
    write_pptx(
        "missing-chart",
        missing_chart,
        default_slide_rels(
            r#"<Relationship Id="rIdMissingChart" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart" Target="../charts/missing.xml"/><Relationship Id="rIdEmptyChart" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart" Target="../charts/empty.xml"/>"#,
        ),
        vec![(
            "ppt/charts/empty.xml".into(),
            br#"<?xml version="1.0" encoding="UTF-8"?>
<c:chartSpace xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart><c:plotArea><c:barChart/></c:plotArea></c:chart></c:chartSpace>"#.to_vec(),
        )],
    );

    let styled_text = slide(
        r#"<p:sp><p:nvSpPr><p:cNvPr id="2" name="Styled text"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x="914400" y="914400"/><a:ext cx="5486400" cy="1828800"/></a:xfrm><a:prstGeom prst="rect"/></p:spPr><p:txBody><a:bodyPr><a:normAutofit fontScale="80000"/></a:bodyPr><a:lstStyle/><a:p><a:r><a:rPr sz="3000" b="1"><a:latin typeface="+mj-lt"/><a:solidFill><a:schemeClr val="tx1"><a:tint val="20000"/></a:schemeClr></a:solidFill></a:rPr><a:t>Hello</a:t></a:r><a:r><a:rPr sz="1800"/><a:t> theme</a:t></a:r></a:p><a:p><a:r><a:rPr sz="1800" i="1"><a:hlinkClick r:id="rIdStyledHyper"/></a:rPr><a:t>Second paragraph</a:t></a:r></a:p></p:txBody></p:sp>"#,
    );
    write_pptx(
        "styled-text",
        styled_text,
        default_slide_rels(
            r#"<Relationship Id="rIdStyledHyper" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="https://example.com/styled-run" TargetMode="External"/>"#,
        ),
        vec![],
    );

    let paths = slide(
        r#"<p:sp><p:nvSpPr><p:cNvPr id="2" name="Rectangle"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x="1270000" y="1270000"/><a:ext cx="2540000" cy="1270000"/></a:xfrm><a:prstGeom prst="rect"/><a:solidFill><a:srgbClr val="AA5500"/></a:solidFill><a:ln w="25400"><a:solidFill><a:srgbClr val="001122"/></a:solidFill></a:ln></p:spPr></p:sp><p:cxnSp><p:nvCxnSpPr><p:cNvPr id="3" name="Connector"/><p:cNvCxnSpPr/><p:nvPr/></p:nvCxnSpPr><p:spPr><a:xfrm><a:off x="3810000" y="1905000"/><a:ext cx="1905000" cy="635000"/></a:xfrm><a:prstGeom prst="line"/><a:ln w="12700"><a:solidFill><a:srgbClr val="CC0000"/></a:solidFill></a:ln></p:spPr></p:cxnSp>"#,
    );
    write_pptx("paths", paths, default_slide_rels(""), vec![]);

    let hyperlink = slide(
        r#"<p:sp><p:nvSpPr><p:cNvPr id="2" name="Link"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x="914400" y="914400"/><a:ext cx="3657600" cy="914400"/></a:xfrm><a:prstGeom prst="rect"/></p:spPr><p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:rPr sz="1800"><a:hlinkClick r:id="rIdHyper"/></a:rPr><a:t>External link</a:t></a:r></a:p></p:txBody></p:sp>"#,
    );
    write_pptx(
        "hyperlink",
        hyperlink,
        default_slide_rels(
            r#"<Relationship Id="rIdHyper" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="https://example.com/literal?x=1&amp;y=2" TargetMode="External"/>"#,
        ),
        vec![],
    );

    // Security corpus. Every member is deterministic and intentionally tiny.
    write_zip(
        "testdata/malformed/not-pptx.zip",
        &[("readme.txt".into(), b"ordinary zip".to_vec())],
        CompressionMethod::Stored,
    );
    write_zip(
        "testdata/malformed/path-traversal.pptx",
        &[("../escape".into(), b"blocked".to_vec())],
        CompressionMethod::Stored,
    );
    write_zip(
        "testdata/malformed/zip-bomb.pptx",
        &[("ppt/presentation.xml".into(), vec![b'A'; 1024 * 1024])],
        CompressionMethod::Deflated,
    );
    fs::write(
        "testdata/malformed/legacy-office.cfb",
        [b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1".as_slice(), b"fixture"].concat(),
    )
    .unwrap();

    let basic_bytes = fs::read("testdata/pptx/basic.pptx").unwrap();
    fs::write(
        "testdata/malformed/truncated.pptx",
        &basic_bytes[..basic_bytes.len() * 2 / 3],
    )
    .unwrap();

    let xxe_slide = slide(
        r#"<p:sp><p:nvSpPr><p:cNvPr id="2" name="XXE"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x="127000" y="127000"/><a:ext cx="1270000" cy="635000"/></a:xfrm><a:prstGeom prst="rect"/></p:spPr><p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:rPr sz="1800"/><a:t>&xxe;</a:t></a:r></a:p></p:txBody></p:sp>"#,
    );
    let internal_xxe = xxe_slide.replacen(
        "?>",
        "?><!DOCTYPE p:sld [<!ENTITY xxe \"EXPANSION_MUST_NOT_APPEAR\">]>",
        1,
    );
    write_pptx(
        "../malformed/xxe",
        internal_xxe,
        default_slide_rels(""),
        vec![],
    );
    let external_xxe = xxe_slide.replacen(
        "?>",
        "?><!DOCTYPE p:sld [<!ENTITY xxe SYSTEM \"file:///etc/passwd\">]>",
        1,
    );
    write_pptx(
        "../malformed/external-entity",
        external_xxe,
        default_slide_rels(""),
        vec![],
    );

    println!("PPTX fixtures written to testdata/pptx and testdata/malformed");
}
