use docray_core::{sniff_format, ExtractError, Format};
use docray_model::Granularity;

#[test]
fn detects_pdf_magic_at_start() {
    assert_eq!(sniff_format(b"%PDF-1.7 rest"), Some(Format::Pdf));
}

#[test]
fn detects_pdf_magic_with_leading_junk_within_1024_bytes() {
    let mut bytes = vec![b'x'; 100];
    bytes.extend_from_slice(b"%PDF-1.4");
    assert_eq!(sniff_format(&bytes), Some(Format::Pdf));
}

#[test]
fn rejects_non_pdf_and_junk_beyond_1024() {
    assert_eq!(sniff_format(b"PK\x03\x04 not a pdf"), None);
    let mut bytes = vec![b'x'; 2000];
    bytes.extend_from_slice(b"%PDF-1.4");
    assert_eq!(sniff_format(&bytes), None);
    assert_eq!(sniff_format(b""), None);
}

// Boundary: the header START must be within the first 1024 bytes. A header
// starting at offset 1023 is accepted; one starting at offset 1024 is not.
#[test]
fn header_start_offset_boundary_is_1024() {
    let marker = b"%PDF-1.7";

    let mut at_1023 = vec![b'x'; 1023];
    at_1023.extend_from_slice(marker);
    assert_eq!(
        sniff_format(&at_1023),
        Some(Format::Pdf),
        "header starting at offset 1023 must be found"
    );

    let mut at_1024 = vec![b'x'; 1024];
    at_1024.extend_from_slice(marker);
    assert_eq!(
        sniff_format(&at_1024),
        None,
        "header starting at offset 1024 must be rejected"
    );
}

#[test]
fn error_codes_are_stable_strings() {
    assert_eq!(ExtractError::UnsupportedFormat.code(), "unsupported_format");
    assert_eq!(ExtractError::EncryptedPdf.code(), "encrypted_pdf");
    assert_eq!(
        ExtractError::TooManyPages {
            limit: 200,
            actual: 300
        }
        .code(),
        "too_many_pages"
    );
    assert_eq!(
        ExtractError::ParseFailure("x".into()).code(),
        "parse_failure"
    );
    assert_eq!(ExtractError::Io("x".into()).code(), "io_error");
    assert_eq!(
        ExtractError::GranularityUnavailable {
            requested: Granularity::Word,
            finest: Granularity::Element,
        }
        .code(),
        "granularity_unavailable"
    );
}
