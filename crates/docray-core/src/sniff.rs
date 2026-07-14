#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Pdf,
}

/// PDF spec allows the `%PDF-` header to *start* anywhere within the first 1024
/// bytes. The marker is 5 bytes, so a header starting at offset 1023 ends at
/// 1027; we therefore scan the first 1024+4 bytes and accept only matches whose
/// start offset is < 1024.
pub fn sniff_format(bytes: &[u8]) -> Option<Format> {
    let window = &bytes[..bytes.len().min(1024 + 4)];
    window
        .windows(5)
        .position(|w| w == b"%PDF-")
        .filter(|&start| start < 1024)
        .map(|_| Format::Pdf)
}
