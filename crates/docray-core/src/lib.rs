mod error;
pub mod grouping;
mod sniff;

pub use error::ExtractError;
pub use sniff::{sniff_format, Format};

use docray_model::Extraction;

pub trait Extractor {
    fn extract(&self, bytes: &[u8], max_pages: Option<u32>) -> Result<Extraction, ExtractError>;
}
