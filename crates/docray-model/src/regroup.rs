//! Detection of glyph-fragmented pages: PDFs where text extraction produced
//! one `Element::Text` per glyph instead of per word/line, typically because
//! the source PDF omits usable ToUnicode/word-boundary information.

/// Below this many text elements carrying glyph geometry, don't bother
/// classifying the page as fragmented — too small a sample to be confident,
/// and tiny pages (e.g. a lone heading) are cheap to leave alone either way.
const FRAGMENT_MIN_ELEMENTS: usize = 8;

/// A page is considered fragmented when at least this fraction of its
/// glyph-bearing text elements are single-glyph.
const FRAGMENT_SINGLE_GLYPH_RATIO: f64 = 0.6;

use crate::Element;

/// Returns true if `elements` looks like a glyph-fragmented text page: many
/// `Element::Text` items each carrying exactly one glyph in their `lines`
/// hierarchy, rather than whole words/lines.
// Not yet called outside tests: a follow-up task wires this into the
// regrouping pipeline. Remove this `allow` once that lands.
#[allow(dead_code)]
pub(crate) fn is_glyph_fragmented(elements: &[Element]) -> bool {
    let mut with_geometry = 0usize;
    let mut single_glyph = 0usize;
    for el in elements {
        if let Element::Text(t) = el {
            if let Some(lines) = &t.lines {
                let glyphs: usize = lines
                    .iter()
                    .flat_map(|l| &l.words)
                    .map(|w| w.chars.len())
                    .sum();
                if glyphs == 0 {
                    continue;
                }
                with_geometry += 1;
                if glyphs == 1 {
                    single_glyph += 1;
                }
            }
        }
    }
    with_geometry >= FRAGMENT_MIN_ELEMENTS
        && (single_glyph as f64) >= FRAGMENT_SINGLE_GLYPH_RATIO * (with_geometry as f64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BBox, Char, Font, Line, TextColor, TextElement, Word};

    fn bbox() -> BBox {
        BBox {
            x0: 0.0,
            y0: 0.0,
            x1: 10.0,
            y1: 10.0,
        }
    }

    fn font() -> Font {
        Font {
            name: "Test".to_string(),
            size: 12.0,
            bold: false,
            italic: false,
        }
    }

    fn color() -> TextColor {
        TextColor {
            fill: Some([0, 0, 0]),
            stroke: None,
        }
    }

    /// Builds one `Element::Text` per entry in `contents`, with a full
    /// line/word/char hierarchy populated (splitting each entry's words on
    /// spaces, one `Char` per character) so `is_glyph_fragmented` sees real
    /// glyph counts.
    fn make_text_page(contents: &[&str]) -> Vec<Element> {
        contents
            .iter()
            .enumerate()
            .map(|(i, content)| {
                let words: Vec<Word> = content
                    .split(' ')
                    .map(|word_str| {
                        let chars: Vec<Char> = word_str
                            .chars()
                            .map(|c| Char {
                                content: c.to_string(),
                                bbox: bbox(),
                                unicode: c as u32,
                            })
                            .collect();
                        Word {
                            content: word_str.to_string(),
                            bbox: bbox(),
                            chars,
                        }
                    })
                    .collect();
                let line = Line {
                    bbox: bbox(),
                    baseline_y: 5.0,
                    words,
                };
                Element::Text(TextElement {
                    id: format!("t{i}"),
                    bbox: bbox(),
                    content: content.to_string(),
                    font: font(),
                    color: color(),
                    lines: Some(vec![line]),
                    runs: None,
                })
            })
            .collect()
    }

    #[test]
    fn ten_single_glyph_elements_is_fragmented() {
        assert!(is_glyph_fragmented(&make_text_page(&[
            "a", "b", "c", "d", "e", "f", "g", "h", "i", "j"
        ])));
    }

    #[test]
    fn ten_multi_word_elements_is_not_fragmented() {
        assert!(!is_glyph_fragmented(&make_text_page(&["hello world"; 10])));
    }

    #[test]
    fn below_min_elements_is_not_fragmented() {
        assert!(!is_glyph_fragmented(&make_text_page(&["a", "b", "c"])));
    }
}
