use clap::{Parser, Subcommand};
use docray_core::{check_granularity, sniff_format, ExtractError, Extractor, Format};
use docray_model::{GranularExtraction, Granularity, OutputFormat};
use docray_pdf::PdfExtractor;
use docray_pptx::PptxExtractor;
use std::process::ExitCode;
use std::str::FromStr;

#[derive(Parser)]
#[command(name = "dps", about = "Document parsing service CLI")]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Extract a document to JSON or lean text on stdout.
    Extract {
        file: String,
        #[arg(long)]
        max_pages: Option<u32>,
        #[arg(long)]
        pretty: bool,
        /// Output detail: element, word, or char. Omit for byte-identical v1.1 output.
        #[arg(long, value_parser = parse_granularity)]
        granularity: Option<Granularity>,
        /// Output encoding: json or lean. Lean implies element granularity.
        #[arg(long, default_value = "json", value_name = "json|lean")]
        format: String,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Cmd::Extract {
            file,
            max_pages,
            pretty,
            granularity,
            format,
        } => run_extract(&file, max_pages, pretty, granularity, &format),
    }
}

fn parse_granularity(value: &str) -> Result<Granularity, String> {
    value.parse()
}

fn run_extract(
    file: &str,
    max_pages: Option<u32>,
    pretty: bool,
    granularity: Option<Granularity>,
    format: &str,
) -> ExitCode {
    let format = match OutputFormat::from_str(format) {
        Ok(format) => format,
        Err(message) => return fail_bad_format(&message),
    };
    let granularity = match (format, granularity) {
        (OutputFormat::Lean, None) => Some(Granularity::Element),
        (OutputFormat::Lean, Some(Granularity::Char)) => {
            return fail_bad_format("lean format requires element or word granularity")
        }
        (_, granularity) => granularity,
    };

    let bytes = match std::fs::read(file) {
        Ok(b) => b,
        Err(e) => return fail(&ExtractError::Io(format!("{file}: {e}"))),
    };
    let result = match sniff_format(&bytes) {
        Some(Format::Pdf) => {
            let extractor = PdfExtractor;
            check_granularity(&extractor.capabilities(), granularity)
                .and_then(|()| extractor.extract(&bytes, max_pages))
        }
        Some(Format::Zip) => {
            let extractor = PptxExtractor;
            check_granularity(&extractor.capabilities(), granularity)
                .and_then(|()| extractor.extract(&bytes, max_pages))
        }
        None if bytes.starts_with(b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1") => {
            PptxExtractor.extract(&bytes, max_pages)
        }
        None => Err(ExtractError::UnsupportedFormat),
    };
    match result {
        Ok(extraction) => {
            match format {
                OutputFormat::Lean => {
                    let compact = match extraction
                        .with_granularity(granularity.expect("lean granularity is validated above"))
                    {
                        GranularExtraction::Compact(compact) => compact,
                        GranularExtraction::Char(_) => {
                            unreachable!("lean char granularity is rejected above")
                        }
                    };
                    print!("{}", compact.to_lean());
                }
                OutputFormat::Json => {
                    let json = if let Some(level) = granularity {
                        if pretty {
                            serde_json::to_string_pretty(&extraction.with_granularity(level))
                        } else {
                            serde_json::to_string(&extraction.with_granularity(level))
                        }
                    } else if pretty {
                        serde_json::to_string_pretty(&extraction)
                    } else {
                        serde_json::to_string(&extraction)
                    }
                    .expect("model serialization cannot fail");
                    println!("{json}");
                }
            }
            ExitCode::SUCCESS
        }
        Err(e) => fail(&e),
    }
}

fn fail_bad_format(message: &str) -> ExitCode {
    eprintln!(
        "{}",
        serde_json::json!({ "error": { "code": "bad_format", "message": message } })
    );
    ExitCode::from(7)
}

fn fail(e: &ExtractError) -> ExitCode {
    eprintln!(
        "{}",
        serde_json::json!({ "error": { "code": e.code(), "message": e.to_string() } })
    );
    ExitCode::from(extract_error_exit_code(e))
}

fn extract_error_exit_code(e: &ExtractError) -> u8 {
    match e {
        ExtractError::UnsupportedFormat | ExtractError::UnsupportedFormatMessage(_) => 2,
        ExtractError::EncryptedPdf => 3,
        ExtractError::ParseFailure(_) => 4,
        ExtractError::Io(_) => 5,
        ExtractError::TooManyPages { .. } => 6,
        ExtractError::GranularityUnavailable { .. } => 8,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn granularity_unavailable_maps_to_exit_8() {
        let error = ExtractError::GranularityUnavailable {
            requested: Granularity::Word,
            finest: Granularity::Element,
        };
        assert_eq!(extract_error_exit_code(&error), 8);
    }
}
