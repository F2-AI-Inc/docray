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

use std::collections::HashMap;

use crate::grouping::{group_into_lines, RawChar};
use crate::{round3, BBox, Element, Font, Line, TextColor, TextElement};

/// Deterministic, hashable identity for a single glyph: its bbox scaled by
/// 1000 and rounded (via `round3`) plus its content. Two glyphs never share
/// this key on a real page, so it's safe to use as a style lookup — but it
/// must never be iterated for output, only looked up, since map iteration
/// order is not deterministic.
pub(crate) type StyleKey = ([i64; 4], String);

/// Per-glyph style, keyed by `StyleKey`, for reconstructing `TextRun`s after
/// regrouping. Never iterate this map for output — glyph identity ordering
/// is not the same as reading order — only look it up by key.
pub(crate) type StyleMap = HashMap<StyleKey, (Font, TextColor, Option<String>)>;

fn style_key(bbox: &BBox, content: &str) -> StyleKey {
    let scale = |v: f64| (round3(v) * 1000.0) as i64;
    (
        [
            scale(bbox.x0),
            scale(bbox.y0),
            scale(bbox.x1),
            scale(bbox.y1),
        ],
        content.to_string(),
    )
}

/// The href carried by a text element's single run, if it has exactly one.
/// Multi-run elements (mixed styling within one element) don't have a
/// single unambiguous href to attribute to each glyph, so they get `None`.
fn href_of(t: &TextElement) -> Option<String> {
    match t.runs.as_deref() {
        Some([run]) => run.href.clone(),
        _ => None,
    }
}

/// Collects every glyph from all `Element::Text` items with glyph geometry
/// on a page, sorts them into a deterministic reading order, and regroups
/// them into `Line`s via `group_into_lines`. This is the fix for
/// glyph-fragmented pages (see `is_glyph_fragmented`): instead of trusting
/// each element's own single-glyph `lines`, every glyph on the page is
/// pooled and re-grouped together.
///
/// Returns the regrouped lines plus a style map (keyed by glyph identity)
/// so a later pass can reconstruct `TextRun`s with the correct font, color,
/// and href per glyph. The map must only ever be looked up by key — never
/// iterated — since `HashMap` iteration order is not deterministic.
// Not yet called outside tests: a follow-up task wires this into the
// regrouping pipeline. Remove this `allow` once that lands.
#[allow(dead_code)]
pub(crate) fn regroup_page_lines(elements: &[Element]) -> (Vec<Line>, StyleMap) {
    let mut style_map: StyleMap = HashMap::new();
    let mut raw_chars: Vec<RawChar> = Vec::new();

    for el in elements {
        let Element::Text(t) = el else { continue };
        let Some(source_lines) = &t.lines else {
            continue;
        };
        let href = href_of(t);
        for source_line in source_lines {
            for word in &source_line.words {
                for ch in &word.chars {
                    style_map.insert(
                        style_key(&ch.bbox, &ch.content),
                        (t.font.clone(), t.color.clone(), href.clone()),
                    );
                    raw_chars.push(RawChar {
                        content: ch.content.clone(),
                        bbox: ch.bbox,
                        unicode: ch.unicode,
                        font_size: t.font.size,
                        baseline_y: source_line.baseline_y,
                    });
                }
            }
        }
    }

    // Total order: baseline (top to bottom), then x0 (left to right), then
    // content and original index as stable tiebreaks. Scaling through
    // `round3` and truncating to `i64` avoids float comparison pitfalls
    // (NaN, unstable partial_cmp) while keeping the order fully deterministic.
    let mut indices: Vec<usize> = (0..raw_chars.len()).collect();
    indices.sort_by(|&a, &b| {
        let ka = &raw_chars[a];
        let kb = &raw_chars[b];
        let sort_tuple = |c: &RawChar, idx: usize| {
            (
                (round3(c.baseline_y) * 1000.0) as i64,
                (round3(c.bbox.x0) * 1000.0) as i64,
                c.content.clone(),
                idx,
            )
        };
        sort_tuple(ka, a).cmp(&sort_tuple(kb, b))
    });
    let sorted: Vec<RawChar> = indices
        .into_iter()
        .map(|i| RawChar {
            content: raw_chars[i].content.clone(),
            bbox: raw_chars[i].bbox,
            unicode: raw_chars[i].unicode,
            font_size: raw_chars[i].font_size,
            baseline_y: raw_chars[i].baseline_y,
        })
        .collect();

    let lines = group_into_lines(&sorted);
    (lines, style_map)
}

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
    use crate::{round3, BBox, Char, Font, Line, TextColor, TextElement, Word};

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

    #[test]
    fn seven_single_glyph_elements_below_min_is_not_fragmented() {
        // 7 < FRAGMENT_MIN_ELEMENTS (8) → not fragmented even though all single-glyph
        assert!(!is_glyph_fragmented(&make_text_page(&[
            "a", "b", "c", "d", "e", "f", "g"
        ])));
    }

    #[test]
    fn eight_single_glyph_elements_at_min_is_fragmented() {
        // exactly at the 8-element floor, all single-glyph → fragmented
        assert!(is_glyph_fragmented(&make_text_page(&[
            "a", "b", "c", "d", "e", "f", "g", "h"
        ])));
    }

    /// Builds one single-glyph `Element::Text` per `(char, baseline_y, x0)`
    /// entry, simulating a glyph-fragmented page. Glyph width is fixed at 8pt
    /// so consecutive same-word glyphs touch (gap 0) while word gaps (12pt)
    /// exceed the `0.25 * font_size` (2.5pt) word-split threshold. The two
    /// baselines (100 vs 120) are 20pt apart, exceeding the `0.5 * font_size`
    /// (5pt) line-split threshold, so they land on separate lines.
    fn make_glyph_page(glyphs: &[(char, f64, f64)]) -> Vec<Element> {
        glyphs
            .iter()
            .enumerate()
            .map(|(i, &(ch, baseline_y, x0))| {
                let (y0, y1) = if baseline_y == 100.0 {
                    (88.0, 100.0)
                } else {
                    (108.0, 120.0)
                };
                let bbox = BBox {
                    x0,
                    y0,
                    x1: x0 + 8.0,
                    y1,
                };
                let char = Char {
                    content: ch.to_string(),
                    bbox,
                    unicode: ch as u32,
                };
                let word = Word {
                    content: ch.to_string(),
                    bbox,
                    chars: vec![char],
                };
                let line = Line {
                    bbox,
                    baseline_y,
                    words: vec![word],
                };
                Element::Text(TextElement {
                    id: format!("g{i}"),
                    bbox,
                    content: ch.to_string(),
                    font: font(),
                    color: color(),
                    lines: Some(vec![line]),
                    runs: None,
                })
            })
            .collect()
    }

    fn test_style_key(bbox: &BBox, content: &str) -> StyleKey {
        let scale = |v: f64| (round3(v) * 1000.0) as i64;
        (
            [
                scale(bbox.x0),
                scale(bbox.y0),
                scale(bbox.x1),
                scale(bbox.y1),
            ],
            content.to_string(),
        )
    }

    #[test]
    fn regroups_shuffled_glyphs_into_two_lines_with_style_map() {
        // "hello world" (baseline 100) and "foo bar" (baseline 120),
        // fed in shuffled order to prove the sort — not input order —
        // determines reading order.
        let elements = make_glyph_page(&[
            ('w', 100.0, 52.0),
            ('f', 120.0, 0.0),
            ('l', 100.0, 24.0),
            ('o', 120.0, 16.0),
            ('h', 100.0, 0.0),
            ('r', 120.0, 52.0),
            ('l', 100.0, 16.0),
            ('d', 100.0, 84.0),
            ('a', 120.0, 44.0),
            ('o', 100.0, 60.0),
            ('e', 100.0, 8.0),
            ('b', 120.0, 36.0),
            ('r', 100.0, 68.0),
            ('o', 120.0, 8.0),
            ('l', 100.0, 76.0),
            ('o', 100.0, 32.0),
        ]);

        let (lines, style_map) = regroup_page_lines(&elements);

        assert_eq!(
            lines.len(),
            2,
            "expected two distinct baselines to become two lines"
        );

        let words0: Vec<&str> = lines[0].words.iter().map(|w| w.content.as_str()).collect();
        assert_eq!(words0, vec!["hello", "world"]);

        let words1: Vec<&str> = lines[1].words.iter().map(|w| w.content.as_str()).collect();
        assert_eq!(words1, vec!["foo", "bar"]);

        // Every glyph's source style must be recoverable from the map.
        assert_eq!(style_map.len(), elements.len());
        for el in &elements {
            let Element::Text(t) = el else { unreachable!() };
            let src_line = t.lines.as_ref().unwrap().first().unwrap();
            let ch = &src_line.words[0].chars[0];
            let key = test_style_key(&ch.bbox, &ch.content);
            let (stored_font, stored_color, stored_href) = style_map
                .get(&key)
                .unwrap_or_else(|| panic!("missing style_map entry for glyph {:?}", ch.content));
            assert_eq!(*stored_font, t.font);
            assert_eq!(*stored_color, t.color);
            assert_eq!(*stored_href, None);
        }

        let (lines_again, _) = regroup_page_lines(&elements);
        assert_eq!(
            lines, lines_again,
            "regroup_page_lines must be deterministic"
        );
    }
}
