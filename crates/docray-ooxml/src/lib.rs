mod package;
mod relationships;
mod xml;

pub use package::{Package, MAX_COMPRESSION_RATIO, MAX_ENTRIES, MAX_ENTRY_SIZE, MAX_TOTAL_SIZE};
pub use relationships::{relationships, resolve_target, Relationship, Relationships};
pub use xml::{local_name, parse, Descendants, Node, MAX_XML_DEPTH, MAX_XML_NODES};

use docray_core::ExtractError;

pub const EMU_PER_POINT: f64 = 12_700.0;
pub const TWIPS_PER_POINT: f64 = 20.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpcKind {
    Pptx,
    Docx,
    OtherZip,
}

pub fn sniff_opc(bytes: &[u8]) -> Result<OpcKind, ExtractError> {
    let package = Package::open(bytes)?;
    if package.contains("ppt/presentation.xml") {
        Ok(OpcKind::Pptx)
    } else if package.contains("word/document.xml") {
        Ok(OpcKind::Docx)
    } else {
        Ok(OpcKind::OtherZip)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Write};
    use zip::write::SimpleFileOptions;
    use zip::ZipWriter;

    fn opc(parts: &[(&str, &[u8])]) -> Vec<u8> {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        for (name, bytes) in parts {
            writer
                .start_file(*name, SimpleFileOptions::default())
                .unwrap();
            writer.write_all(bytes).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    #[test]
    fn sniff_opc_classifies_pptx_and_pptm_by_main_part() {
        let pptx = opc(&[("ppt/presentation.xml", b"<p:presentation/>")]);
        assert_eq!(sniff_opc(&pptx).unwrap(), OpcKind::Pptx);

        let pptm = opc(&[
            ("ppt/presentation.xml", b"<p:presentation/>"),
            (
                "[Content_Types].xml",
                b"<Types><Override PartName=\"/ppt/presentation.xml\" ContentType=\"application/vnd.ms-powerpoint.presentation.macroEnabled.main+xml\"/></Types>",
            ),
            ("ppt/vbaProject.bin", b"macros are not inspected"),
        ]);
        assert_eq!(sniff_opc(&pptm).unwrap(), OpcKind::Pptx);
    }

    #[test]
    fn sniff_opc_classifies_docx_and_docm_by_main_part() {
        let docx = opc(&[("word/document.xml", b"<w:document/>")]);
        assert_eq!(sniff_opc(&docx).unwrap(), OpcKind::Docx);

        let docm = opc(&[
            ("word/document.xml", b"<w:document/>"),
            (
                "[Content_Types].xml",
                b"<Types><Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.ms-word.document.macroEnabled.main+xml\"/></Types>",
            ),
            ("word/vbaProject.bin", b"macros are not inspected"),
        ]);
        assert_eq!(sniff_opc(&docm).unwrap(), OpcKind::Docx);
    }

    #[test]
    fn sniff_opc_classifies_plain_zip() {
        let zip = opc(&[("notes.txt", b"not an OOXML package")]);
        assert_eq!(sniff_opc(&zip).unwrap(), OpcKind::OtherZip);
    }

    #[test]
    fn sniff_opc_applies_package_security_caps() {
        let hostile = opc(&[("../ppt/presentation.xml", b"<p:presentation/>")]);
        let error = sniff_opc(&hostile).unwrap_err();
        assert_eq!(error.code(), "parse_failure");
        assert!(error.to_string().contains("unsafe OPC entry name rejected"));
    }
}
