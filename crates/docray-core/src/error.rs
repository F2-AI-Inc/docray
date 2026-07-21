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
        }
    }
}
