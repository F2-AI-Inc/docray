//! Pixel size of JPEG 2000 (`JPXDecode`) image data, read from the codestream's
//! SIZ marker segment (ISO/IEC 15444-1 A.5.1) without decoding it.

const JP2_SIGNATURE: [u8; 12] = [0, 0, 0, 12, b'j', b'P', b' ', b' ', 0x0D, 0x0A, 0x87, 0x0A];

/// Returns the image size `(Xsiz - XOsiz, Ysiz - YOsiz)` of a raw codestream or
/// of a JP2 file's top-level `jp2c` box, or `None` if the header is absent or
/// malformed.
pub(crate) fn pixel_size(data: &[u8]) -> Option<(u32, u32)> {
    if data.starts_with(&JP2_SIGNATURE) {
        codestream_size(jp2_codestream(data)?)
    } else {
        codestream_size(data)
    }
}

fn jp2_codestream(mut data: &[u8]) -> Option<&[u8]> {
    while data.len() >= 8 {
        let kind = &data[4..8];
        let (header, length) = match u32::from_be_bytes(data[..4].try_into().ok()?) {
            0 => (8, data.len() as u64),
            1 => (16, u64::from_be_bytes(data.get(8..16)?.try_into().ok()?)),
            length => (8, u64::from(length)),
        };
        if length < header as u64 || length > data.len() as u64 {
            return None;
        }
        let (contents, rest) = data.split_at(length as usize);
        if kind == b"jp2c" {
            return Some(&contents[header..]);
        }
        data = rest;
    }
    None
}

fn codestream_size(codestream: &[u8]) -> Option<(u32, u32)> {
    // SIZ must immediately follow SOC: marker, Lsiz, Rsiz, Xsiz, Ysiz, XOsiz, YOsiz.
    if codestream.get(..4)? != [0xFF, 0x4F, 0xFF, 0x51] {
        return None;
    }
    let field = |at: usize| -> Option<u32> {
        Some(u32::from_be_bytes(
            codestream.get(at..at + 4)?.try_into().ok()?,
        ))
    };
    let width = field(8)?.checked_sub(field(16)?)?;
    let height = field(12)?.checked_sub(field(20)?)?;
    (width > 0 && height > 0).then_some((width, height))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn codestream(x: u32, y: u32, x_offset: u32, y_offset: u32) -> Vec<u8> {
        let mut cs = vec![0xFF, 0x4F, 0xFF, 0x51, 0x00, 0x29, 0x00, 0x00];
        for value in [x, y, x_offset, y_offset] {
            cs.extend(value.to_be_bytes());
        }
        cs
    }

    fn jp2_box(kind: &[u8; 4], contents: &[u8]) -> Vec<u8> {
        let mut b = ((contents.len() + 8) as u32).to_be_bytes().to_vec();
        b.extend(kind);
        b.extend(contents);
        b
    }

    #[test]
    fn raw_codestream_size_excludes_the_image_offset() {
        assert_eq!(pixel_size(&codestream(640, 480, 0, 0)), Some((640, 480)));
        assert_eq!(pixel_size(&codestream(110, 70, 10, 20)), Some((100, 50)));
    }

    #[test]
    fn jp2_size_comes_from_the_jp2c_box_after_other_boxes() {
        let mut file = JP2_SIGNATURE.to_vec();
        file.extend(jp2_box(b"ftyp", b"jp2 \0\0\0\0jp2 "));
        file.extend(jp2_box(b"jp2h", &jp2_box(b"ihdr", &[0; 14])));
        file.extend(jp2_box(b"jp2c", &codestream(3, 2, 0, 0)));
        assert_eq!(pixel_size(&file), Some((3, 2)));
    }

    #[test]
    fn jp2_box_lengths_of_zero_and_one_are_honoured() {
        let mut to_end = JP2_SIGNATURE.to_vec();
        to_end.extend([0, 0, 0, 0]);
        to_end.extend(b"jp2c");
        to_end.extend(codestream(7, 5, 0, 0));
        assert_eq!(pixel_size(&to_end), Some((7, 5)));

        let cs = codestream(9, 4, 0, 0);
        let mut extended = JP2_SIGNATURE.to_vec();
        extended.extend([0, 0, 0, 1]);
        extended.extend(b"jp2c");
        extended.extend(((cs.len() + 16) as u64).to_be_bytes());
        extended.extend(cs);
        assert_eq!(pixel_size(&extended), Some((9, 4)));
    }

    #[test]
    fn malformed_headers_have_no_size() {
        let full = codestream(8, 8, 0, 0);
        assert_eq!(pixel_size(&full[..23]), None, "truncated SIZ");
        assert_eq!(pixel_size(&full[2..]), None, "missing SOC");
        assert_eq!(
            pixel_size(&codestream(8, 8, 9, 0)),
            None,
            "offset past width"
        );
        assert_eq!(pixel_size(&codestream(8, 8, 8, 0)), None, "zero width");
        assert_eq!(pixel_size(b"\xFF\xD8\xFF\xE0"), None, "not JPEG 2000");

        let mut overlong = JP2_SIGNATURE.to_vec();
        overlong.extend([0, 0, 1, 0]);
        overlong.extend(b"jp2c");
        overlong.extend(&full);
        assert_eq!(pixel_size(&overlong), None, "box longer than the data");

        let mut undersized = JP2_SIGNATURE.to_vec();
        undersized.extend([0, 0, 0, 4]);
        undersized.extend(b"jp2c");
        undersized.extend(&full);
        assert_eq!(pixel_size(&undersized), None, "box shorter than its header");

        let mut without_codestream = JP2_SIGNATURE.to_vec();
        without_codestream.extend(jp2_box(b"jp2h", &[0; 22]));
        assert_eq!(pixel_size(&without_codestream), None, "no jp2c box");
    }
}
