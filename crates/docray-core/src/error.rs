use thiserror::Error;

#[derive(Debug, Error)]
pub enum ExtractError {
    #[error("input is not a supported format")]
    UnsupportedFormat,
    #[error("PDF is encrypted / password-protected")]
    EncryptedPdf,
    #[error("document has {actual} pages, limit is {limit}")]
    TooManyPages { limit: u32, actual: u32 },
    #[error("failed to parse document: {0}")]
    ParseFailure(String),
    #[error("io error: {0}")]
    Io(String),
}

impl ExtractError {
    pub fn code(&self) -> &'static str {
        match self {
            ExtractError::UnsupportedFormat => "unsupported_format",
            ExtractError::EncryptedPdf => "encrypted_pdf",
            ExtractError::TooManyPages { .. } => "too_many_pages",
            ExtractError::ParseFailure(_) => "parse_failure",
            ExtractError::Io(_) => "io_error",
        }
    }
}
