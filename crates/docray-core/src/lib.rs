mod error;
pub mod grouping;
mod sniff;

pub use error::ExtractError;
pub use sniff::{sniff_format, Format};

use docray_model::{Extraction, Granularity};

/// Capabilities of an extraction backend. Keeping this extensible as a struct
/// allows future independent capability axes without changing the trait method.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    pub finest_granularity: Granularity,
}

pub trait Extractor {
    fn capabilities(&self) -> Capabilities;

    fn extract(&self, bytes: &[u8], max_pages: Option<u32>) -> Result<Extraction, ExtractError>;
}

/// Rejects a request that needs a finer hierarchy than an extractor provides.
/// An absent request is the frozen full/char-level response.
pub fn check_granularity(
    capabilities: &Capabilities,
    requested: Option<Granularity>,
) -> Result<(), ExtractError> {
    let requested = requested.unwrap_or(Granularity::Char);
    if requested.rank() > capabilities.finest_granularity.rank() {
        Err(ExtractError::GranularityUnavailable {
            requested,
            finest: capabilities.finest_granularity,
        })
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct ElementExtractor;

    impl Extractor for ElementExtractor {
        fn capabilities(&self) -> Capabilities {
            Capabilities {
                finest_granularity: Granularity::Element,
            }
        }

        fn extract(
            &self,
            _bytes: &[u8],
            _max_pages: Option<u32>,
        ) -> Result<Extraction, ExtractError> {
            unreachable!("capability gate tests do not extract")
        }
    }

    #[test]
    fn element_only_extractor_rejects_finer_and_implicit_requests() {
        let capabilities = ElementExtractor.capabilities();
        for (requested, effective) in [
            (None, Granularity::Char),
            (Some(Granularity::Char), Granularity::Char),
            (Some(Granularity::Word), Granularity::Word),
        ] {
            assert_eq!(
                check_granularity(&capabilities, requested),
                Err(ExtractError::GranularityUnavailable {
                    requested: effective,
                    finest: Granularity::Element,
                })
            );
        }
    }

    #[test]
    fn element_only_extractor_accepts_element_requests() {
        // Lean entry points normalize their default to an explicit Element
        // request before reaching this gate, so Element covers lean too.
        let capabilities = ElementExtractor.capabilities();
        assert_eq!(
            check_granularity(&capabilities, Some(Granularity::Element)),
            Ok(())
        );
    }
}
