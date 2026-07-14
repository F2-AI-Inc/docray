use clap::{Parser, Subcommand};
use docray_core::{sniff_format, ExtractError, Extractor, Format};
use docray_model::Granularity;
use docray_pdf::PdfExtractor;
use std::process::ExitCode;

#[derive(Parser)]
#[command(name = "dps", about = "Document parsing service CLI")]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Extract a document to JSON on stdout.
    Extract {
        file: String,
        #[arg(long)]
        max_pages: Option<u32>,
        #[arg(long)]
        pretty: bool,
        /// Output detail: element, word, or char. Omit for byte-identical v1.1 output.
        #[arg(long, value_parser = parse_granularity)]
        granularity: Option<Granularity>,
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
        } => run_extract(&file, max_pages, pretty, granularity),
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
) -> ExitCode {
    let bytes = match std::fs::read(file) {
        Ok(b) => b,
        Err(e) => return fail(&ExtractError::Io(format!("{file}: {e}"))),
    };
    let result = match sniff_format(&bytes) {
        Some(Format::Pdf) => PdfExtractor.extract(&bytes, max_pages),
        None => Err(ExtractError::UnsupportedFormat),
    };
    match result {
        Ok(extraction) => {
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
            ExitCode::SUCCESS
        }
        Err(e) => fail(&e),
    }
}

fn fail(e: &ExtractError) -> ExitCode {
    eprintln!(
        "{}",
        serde_json::json!({ "error": { "code": e.code(), "message": e.to_string() } })
    );
    let code: u8 = match e {
        ExtractError::UnsupportedFormat => 2,
        ExtractError::EncryptedPdf => 3,
        ExtractError::ParseFailure(_) => 4,
        ExtractError::Io(_) => 5,
        ExtractError::TooManyPages { .. } => 6,
    };
    ExitCode::from(code)
}
