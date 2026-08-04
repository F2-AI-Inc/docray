use thiserror::Error;

use docray_model::Granularity;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ExtractError {
    #[error("input is not a supported format")]
    UnsupportedFormat,
    #[error("{0}")]
    UnsupportedFormatMessage(String),
    #[error("PDF is encrypted / password-protected")]
    EncryptedPdf,
    #[error("document has {actual} pages, limit is {limit}")]
    TooManyPages { limit: u32, actual: u32 },
    #[error("failed to parse document: {0}")]
    ParseFailure(String),
    #[error("io error: {0}")]
    Io(String),
    #[error("requested {requested} granularity is unavailable; finest available is {finest}; retry with granularity={finest}")]
    GranularityUnavailable {
        requested: Granularity,
        finest: Granularity,
    },
    #[error("requested pages end at {requested_end} but document has {page_count} pages")]
    PageOutOfRange { requested_end: u32, page_count: u32 },
    #[error("page selection is not supported for {format}")]
    PageSelectionUnsupported { format: &'static str },
}

impl ExtractError {
    pub fn code(&self) -> &'static str {
        match self {
            ExtractError::UnsupportedFormat | ExtractError::UnsupportedFormatMessage(_) => {
                "unsupported_format"
            }
            ExtractError::EncryptedPdf => "encrypted_pdf",
            ExtractError::TooManyPages { .. } => "too_many_pages",
            ExtractError::ParseFailure(_) => "parse_failure",
            ExtractError::Io(_) => "io_error",
            ExtractError::GranularityUnavailable { .. } => "granularity_unavailable",
            ExtractError::PageOutOfRange { .. } => "page_out_of_range",
            ExtractError::PageSelectionUnsupported { .. } => "page_selection_unsupported",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_out_of_range_code_and_display() {
        let err = ExtractError::PageOutOfRange {
            requested_end: 300,
            page_count: 287,
        };
        assert_eq!(err.code(), "page_out_of_range");
        // Must not panic when formatted.
        let _ = err.to_string();
    }

    #[test]
    fn page_selection_unsupported_code_and_display() {
        let err = ExtractError::PageSelectionUnsupported { format: "pptx" };
        assert_eq!(err.code(), "page_selection_unsupported");
        let _ = err.to_string();
    }
}
