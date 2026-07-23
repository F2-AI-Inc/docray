use clap::{Parser, Subcommand};
use docray_core::{check_granularity, sniff_format, Capabilities, ExtractError, Extractor, Format};
use docray_model::{Extraction, GranularExtraction, Granularity, OutputFormat};
use docray_ooxml::{sniff_opc, OpcKind};
use docray_pdf::PdfExtractor;
use docray_pptx::PptxExtractor;
use std::io::Write;
use std::process::ExitCode;
use std::str::FromStr;

/// CFB (OLE2) magic — legacy or encrypted Office documents.
const CFB_MAGIC: &[u8; 8] = b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1";

/// The extraction backend selected by format sniffing.
enum Backend {
    Pdf,
    Pptx,
    Zip,
}

impl Backend {
    fn capabilities(&self) -> Capabilities {
        match self {
            Backend::Pdf => PdfExtractor.capabilities(),
            Backend::Pptx | Backend::Zip => PptxExtractor.capabilities(),
        }
    }

    fn extract(&self, bytes: &[u8], max_pages: Option<u32>) -> Result<Extraction, ExtractError> {
        match self {
            Backend::Pdf => PdfExtractor.extract(bytes, max_pages),
            Backend::Pptx => PptxExtractor.extract(bytes, max_pages),
            Backend::Zip => match sniff_opc(bytes)? {
                OpcKind::Pptx => PptxExtractor.extract(bytes, max_pages),
                OpcKind::Docx | OpcKind::OtherZip => Err(unsupported_zip()),
            },
        }
    }
}

#[derive(Parser)]
#[command(
    name = "docray",
    version,
    about = "docray — X-ray for documents: extract PDF & PPTX to JSON"
)]
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
    let backend = match sniff_format(&bytes) {
        Some(Format::Pdf) => Backend::Pdf,
        Some(Format::Zip) => Backend::Zip,
        None if bytes.starts_with(CFB_MAGIC) => Backend::Pptx,
        None => return fail(&ExtractError::UnsupportedFormat),
    };
    let capabilities = backend.capabilities();
    // A missing granularity defaults to the finest the format supports when
    // that is coarser than char, so `docray extract deck.pptx` yields element
    // output instead of erroring. PDF (finest = char) keeps None -> the frozen
    // v1.1 full-hierarchy response.
    let granularity = match granularity {
        None if capabilities.finest_granularity.rank() < Granularity::Char.rank() => {
            Some(capabilities.finest_granularity)
        }
        other => other,
    };
    let result = check_granularity(&capabilities, granularity)
        .and_then(|()| backend.extract(&bytes, max_pages));
    match result {
        Ok(extraction) => {
            let output = match format {
                OutputFormat::Lean => {
                    match extraction
                        .with_granularity(granularity.expect("lean granularity is validated above"))
                    {
                        GranularExtraction::Compact(compact) => compact.to_lean(),
                        GranularExtraction::Flow(flow) => flow.to_lean(),
                        GranularExtraction::Char(_) => {
                            unreachable!("lean char granularity is rejected above")
                        }
                    }
                }
                OutputFormat::Json => {
                    let mut json = if let Some(level) = granularity {
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
                    json.push('\n');
                    json
                }
            };
            write_stdout(&output)
        }
        Err(e) => fail(&e),
    }
}

fn unsupported_zip() -> ExtractError {
    ExtractError::UnsupportedFormatMessage("zip archive is not a PowerPoint file".into())
}

/// Write extraction output to stdout without panicking on a closed pipe.
///
/// `println!` panics on EPIPE (Rust ignores SIGPIPE), so `docray extract x.pdf
/// | head` would exit 101 with a panic message — outside the stable error
/// contract. A broken pipe means the reader chose to stop and is a quiet
/// success (the convention of cat/grep/ripgrep); any other stdout write
/// failure maps to the documented io_error / exit 5.
fn write_stdout(output: &str) -> ExitCode {
    let mut stdout = std::io::stdout().lock();
    match stdout
        .write_all(output.as_bytes())
        .and_then(|()| stdout.flush())
    {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => ExitCode::SUCCESS,
        Err(e) => fail(&ExtractError::Io(format!("stdout: {e}"))),
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
