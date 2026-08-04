use docray_core::{ExtractError, Extractor, PageSelection};
use docray_pdf::PdfExtractor;

/// cargo runs test binaries with CWD = crate dir, so bind()'s relative
/// `./.pdfium/lib` candidate never resolves against the workspace-root lib.
/// Point the documented env var at the workspace-root .pdfium/lib.
fn ensure_pdfium_dir() {
    if std::env::var_os("DOCRAY_PDFIUM_DIR").is_none() {
        std::env::set_var(
            "DOCRAY_PDFIUM_DIR",
            concat!(env!("CARGO_MANIFEST_DIR"), "/../../.pdfium/lib"),
        );
    }
}

fn multipage_bytes() -> Vec<u8> {
    ensure_pdfium_dir();
    let path = format!(
        "{}/../../testdata/multipage.pdf",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read(path).unwrap()
}

fn sel(spec: &str) -> PageSelection {
    spec.parse().unwrap()
}

#[test]
fn selected_range_yields_exact_absolute_page_numbers() {
    let bytes = multipage_bytes();
    let extraction = PdfExtractor
        .extract(&bytes, None, Some(sel("2-4")))
        .unwrap();
    let page_numbers: Vec<u32> = extraction.pages.iter().map(|p| p.page_number).collect();
    assert_eq!(page_numbers, vec![2, 3, 4]);
}

#[test]
fn split_selection_is_equivalent_to_full_extraction() {
    let bytes = multipage_bytes();
    let full = PdfExtractor.extract(&bytes, None, None).unwrap();

    let first_half = PdfExtractor
        .extract(&bytes, None, Some(sel("1-2")))
        .unwrap();
    let second_half = PdfExtractor
        .extract(&bytes, None, Some(sel("3-6")))
        .unwrap();
    let mut combined = first_half.pages;
    combined.extend(second_half.pages);

    // Deep equality: page_number, element count, and each element's bbox
    // (and everything else) must match the corresponding full-document page.
    assert_eq!(combined, full.pages);
}

#[test]
fn cap_applies_to_selected_count_not_document_total() {
    let bytes = multipage_bytes();
    let err = PdfExtractor
        .extract(&bytes, Some(5), Some(sel("1-6")))
        .unwrap_err();
    assert_eq!(
        err,
        ExtractError::TooManyPages {
            limit: 5,
            actual: 6
        }
    );
}

#[test]
fn full_document_still_respects_max_pages_cap() {
    let bytes = multipage_bytes();
    let err = PdfExtractor.extract(&bytes, Some(5), None).unwrap_err();
    assert_eq!(
        err,
        ExtractError::TooManyPages {
            limit: 5,
            actual: 6
        }
    );
}

#[test]
fn selection_beyond_document_end_is_page_out_of_range() {
    let bytes = multipage_bytes();
    let err = PdfExtractor
        .extract(&bytes, None, Some(sel("7-8")))
        .unwrap_err();
    assert_eq!(
        err,
        ExtractError::PageOutOfRange {
            requested_end: 8,
            page_count: 6
        }
    );
}

#[test]
fn document_page_count_stays_the_document_total_when_selecting() {
    let bytes = multipage_bytes();
    let extraction = PdfExtractor
        .extract(&bytes, None, Some(sel("2-4")))
        .unwrap();
    assert_eq!(extraction.document.page_count, 6);
}

#[test]
fn just_under_cap_passes() {
    let bytes = multipage_bytes();
    let extraction = PdfExtractor
        .extract(&bytes, Some(5), Some(sel("1-5")))
        .unwrap();
    let page_numbers: Vec<u32> = extraction.pages.iter().map(|p| p.page_number).collect();
    assert_eq!(page_numbers, vec![1, 2, 3, 4, 5]);
}

#[test]
fn single_page_selection_through_extractor() {
    let bytes = multipage_bytes();
    let extraction = PdfExtractor
        .extract(&bytes, None, Some(sel("3-3")))
        .unwrap();
    assert_eq!(extraction.pages.len(), 1);
    assert_eq!(extraction.pages[0].page_number, 3);
}
