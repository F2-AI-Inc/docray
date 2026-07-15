use docray_core::Extractor;
use docray_pdf::{bind::pdfium, PdfExtractor};
use serde_json::json;
use std::time::Instant;

fn extract_json(bytes: &[u8]) -> String {
    let extraction = PdfExtractor.extract(bytes, None).expect("extract form.pdf");
    serde_json::to_string(&extraction).expect("serialize extraction")
}

fn main() {
    let mode = std::env::args().nth(1).unwrap_or_else(|| "warm".into());
    let bytes = std::fs::read("testdata/form.pdf").expect("read form.pdf");

    if mode == "cold" {
        let start = Instant::now();
        drop(pdfium().expect("initialize native Pdfium"));
        println!(
            "{}",
            json!({ "init_ms": start.elapsed().as_secs_f64() * 1000.0 })
        );
        return;
    }

    if mode == "cold-extract" {
        let start = Instant::now();
        drop(extract_json(&bytes));
        println!(
            "{}",
            json!({ "extract_ms": start.elapsed().as_secs_f64() * 1000.0 })
        );
        return;
    }

    let first_start = Instant::now();
    drop(extract_json(&bytes));
    let first_extract_ms = first_start.elapsed().as_secs_f64() * 1000.0;
    let mut samples = Vec::with_capacity(100);
    for _ in 0..100 {
        let start = Instant::now();
        drop(extract_json(&bytes));
        samples.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    println!(
        "{}",
        json!({ "first_extract_ms": first_extract_ms, "samples": samples })
    );
}
