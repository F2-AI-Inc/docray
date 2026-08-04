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
use crate::{
    compact_bbox, compact_color, compact_element, compact_font, compact_runs, BBox, Char,
    CompactElement, CompactTextContent, CompactTextElement, CompactWord, Element, Font,
    Granularity, Line, TextColor, TextElement, TextRun,
};

/// Scales a coordinate by 1000 and rounds to the nearest integer, for use as
/// a hashable/orderable proxy for an `f64`. Rounding must happen *after*
/// scaling: rounding first (e.g. via `round3`) and then multiplying can
/// leave a value like `1.005` as `1004.999...`, which truncates to `1004`
/// instead of `1005` under `as i64`. `f64::round` handles the
/// half-away-from-zero case correctly before the truncating cast.
fn scale(v: f64) -> i64 {
    (v * 1000.0).round() as i64
}

/// Deterministic, hashable identity for a single glyph: its bbox scaled by
/// 1000 and rounded (via `scale`) plus its content. Two glyphs never share
/// this key on a real page, so it's safe to use as a style lookup — but it
/// must never be iterated for output, only looked up, since map iteration
/// order is not deterministic.
pub(crate) type StyleKey = ([i64; 4], String);

/// Per-glyph style, keyed by `StyleKey`, for reconstructing `TextRun`s after
/// regrouping. Never iterate this map for output — glyph identity ordering
/// is not the same as reading order — only look it up by key.
pub(crate) type StyleMap = HashMap<StyleKey, (Font, TextColor, Option<String>)>;

fn style_key(bbox: &BBox, content: &str) -> StyleKey {
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
    // content and original index as stable tiebreaks. Scaling via `scale`
    // (round-then-truncate) avoids float comparison pitfalls (NaN, unstable
    // partial_cmp) while keeping the order fully deterministic.
    let mut indices: Vec<usize> = (0..raw_chars.len()).collect();
    indices.sort_by(|&a, &b| {
        let ka = &raw_chars[a];
        let kb = &raw_chars[b];
        let sort_tuple = |c: &RawChar, idx: usize| {
            (
                scale(c.baseline_y),
                scale(c.bbox.x0),
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

/// A glyph's font/color/href, looked up by its geometric identity in
/// `style_map`. Falls back to a neutral default rather than panicking if the
/// key is ever absent — see the `StyleKey` doc comment on why a genuine miss
/// shouldn't happen (every char reaching here was inserted into the same
/// `style_map` by the `regroup_page_lines` call that produced it) but must
/// never be allowed to panic if a pathological (bbox, content) collision
/// ever overwrote it.
fn char_style(style_map: &StyleMap, ch: &Char) -> (Font, TextColor, Option<String>) {
    style_map
        .get(&style_key(&ch.bbox, &ch.content))
        .cloned()
        .unwrap_or_else(fallback_style)
}

// Intentionally-uncovered defensive branch: no test triggers it, since doing
// so would require an actual (bbox, content) collision inside `style_map`,
// which `regroup_page_lines`'s own construction can't produce from realistic
// input (see `char_style`'s doc comment).
fn fallback_style() -> (Font, TextColor, Option<String>) {
    (
        Font {
            name: String::new(),
            size: 0.0,
            bold: false,
            italic: false,
        },
        TextColor {
            fill: None,
            stroke: None,
        },
        None,
    )
}

/// Walks a regrouped line's words in reading order and builds the `TextRun`
/// sequence over the line's TEXT exactly as `CompactTextContent::Element`
/// renders it (words joined by a single `" "`): before every word except the
/// first, a synthetic space is emitted — styled as the PRECEDING char's
/// style, since that's whichever run it trails — followed by each of the
/// word's chars in its own style. Consecutive entries sharing
/// `(font, color, href)` are merged into one `TextRun`.
///
/// This upholds the same invariant enforced elsewhere in the model (e.g.
/// docray-docx/docray-pptx's `merge_run`): concatenating every run's
/// `content` in order reproduces the element's full text, spaces included.
fn build_runs(line: &Line, style_map: &StyleMap) -> Vec<TextRun> {
    let mut runs: Vec<TextRun> = Vec::new();
    let mut push =
        |content: &str, (font, color, href): (Font, TextColor, Option<String>)| match runs
            .last_mut()
        {
            Some(last) if last.font == font && last.color == color && last.href == href => {
                last.content.push_str(content);
            }
            _ => runs.push(TextRun {
                content: content.to_string(),
                font,
                color,
                href,
            }),
        };

    for (i, word) in line.words.iter().enumerate() {
        if i > 0 {
            if let Some(prev_last_char) = line.words[i - 1].chars.last() {
                push(" ", char_style(style_map, prev_last_char));
            }
        }
        for ch in &word.chars {
            push(&ch.content, char_style(style_map, ch));
        }
    }
    runs
}

/// Projects one regrouped `Line` to a `CompactElement::Text`: content shaped
/// per `granularity`, font/color taken from the line's first char, and
/// `runs` reconstructed by `build_runs` — but only when the line actually has
/// intra-line style variation. A line whose glyphs (and inserted spaces) are
/// all one style collapses `build_runs` to a single run equal to the
/// element-level font/color, so `runs` is elided to `None` there, matching
/// how the existing compact path represents single-style text.
fn compact_line(line: &Line, granularity: Granularity, style_map: &StyleMap) -> CompactElement {
    let content = match granularity {
        Granularity::Element => CompactTextContent::Element {
            text: line
                .words
                .iter()
                .map(|w| w.content.as_str())
                .collect::<Vec<_>>()
                .join(" "),
        },
        Granularity::Word => CompactTextContent::Word {
            words: line
                .words
                .iter()
                .map(|w| {
                    let [x0, y0, x1, y1] = compact_bbox(&w.bbox);
                    CompactWord(w.content.clone(), x0, y0, x1, y1)
                })
                .collect(),
        },
        Granularity::Char => unreachable!("char granularity does not use compact elements"),
    };

    let (font, color, _href) = line
        .words
        .first()
        .and_then(|w| w.chars.first())
        .map(|ch| char_style(style_map, ch))
        .unwrap_or_else(fallback_style);

    let runs = build_runs(line, style_map);
    let runs = if runs.len() > 1 { Some(runs) } else { None };

    CompactElement::Text(CompactTextElement {
        bbox: compact_bbox(&line.bbox),
        content,
        font: compact_font(&font),
        color: compact_color(&color),
        runs: compact_runs(&runs),
    })
}

/// The compact bbox of any `CompactElement` variant, for reading-order sort.
fn compact_element_bbox(element: &CompactElement) -> [f64; 4] {
    match element {
        CompactElement::Text(t) => t.bbox,
        CompactElement::Table(t) => t.bbox,
        CompactElement::Chart(t) => t.bbox,
        CompactElement::Image(t) => t.bbox,
        CompactElement::Path(t) => t.bbox,
        CompactElement::Annotation(t) => t.bbox,
    }
}

/// Builds the compact (element/word granularity) projection of a
/// glyph-fragmented page: text is pooled and regrouped into `Line`s via
/// `regroup_page_lines` and rendered as one `CompactElement::Text` per line
/// (with reconstructed style runs); every non-text element passes through
/// the existing `compact_element` unchanged. The result is a single list in
/// reading order, mixing both kinds.
///
/// Ordering is a total order on `(scale(bbox.y0), scale(bbox.x0))` of the
/// compact bbox, with a stable index tiebreak — never raw float comparison
/// (see `scale`'s doc comment) and never `HashMap` iteration order.
pub(crate) fn compact_fragmented_elements(
    elements: &[Element],
    granularity: Granularity,
) -> Vec<CompactElement> {
    let (lines, style_map) = regroup_page_lines(elements);

    let mut items: Vec<((i64, i64), usize, CompactElement)> = Vec::new();
    let mut next_index = 0usize;

    for line in &lines {
        let compact = compact_line(line, granularity, &style_map);
        let bbox = compact_element_bbox(&compact);
        items.push(((scale(bbox[1]), scale(bbox[0])), next_index, compact));
        next_index += 1;
    }

    for el in elements {
        if matches!(el, Element::Text(_)) {
            continue;
        }
        let compact = compact_element(el, granularity);
        let bbox = compact_element_bbox(&compact);
        items.push(((scale(bbox[1]), scale(bbox[0])), next_index, compact));
        next_index += 1;
    }

    items.sort_by_key(|a| (a.0, a.1));
    items.into_iter().map(|(_, _, el)| el).collect()
}

/// Returns true if `elements` looks like a glyph-fragmented text page: many
/// `Element::Text` items each carrying exactly one glyph in their `lines`
/// hierarchy, rather than whole words/lines.
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
    use crate::{BBox, Char, Font, ImageElement, Line, TextColor, TextElement, Word};

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

    #[test]
    fn scale_rounds_instead_of_truncating() {
        // Regression test: the old `(round3(v) * 1000.0) as i64` truncated
        // after scaling, so e.g. `round3(1.005) * 1000.0 == 1004.999...`
        // cast (truncated) to 1004, not 1005 — and `round3(2.03) * 1000.0
        // == 2029.999...` truncated to 2029, not 2030. `scale` must round
        // *after* multiplying by 1000, giving the exact values.
        assert_eq!(scale(1.005), 1005);
        assert_eq!(scale(2.03), 2030);
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
            let key = style_key(&ch.bbox, &ch.content);
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

    /// Same shuffled "hello world" / "foo bar" glyph soup as
    /// `regroups_shuffled_glyphs_into_two_lines_with_style_map`, plus one
    /// `Image` element sitting between the two text baselines (y0=104,
    /// between line0's y1=100 and line1's y0=108) so the reading-order sort
    /// must interleave a non-text element between two regrouped text lines
    /// — not just place all text before all non-text.
    fn fragmented_page_with_image() -> Vec<Element> {
        let mut elements = make_glyph_page(&[
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
        elements.push(Element::Image(ImageElement {
            id: "img0".to_string(),
            bbox: BBox {
                x0: 0.0,
                y0: 104.0,
                x1: 20.0,
                y1: 106.0,
            },
            quad: [[0.0, 104.0], [20.0, 104.0], [20.0, 106.0], [0.0, 106.0]],
            pixel_width: Some(10),
            pixel_height: Some(10),
            colorspace: None,
            content_hash: None,
        }));
        elements
    }

    #[test]
    fn compact_fragmented_elements_element_granularity_orders_text_and_image() {
        let elements = fragmented_page_with_image();

        let compact = compact_fragmented_elements(&elements, Granularity::Element);
        assert_eq!(compact.len(), 3, "two text lines + one image");

        let CompactElement::Text(first) = &compact[0] else {
            panic!("expected first (topmost) element to be text")
        };
        match &first.content {
            CompactTextContent::Element { text } => assert_eq!(text, "hello world"),
            CompactTextContent::Word { .. } => panic!("expected element-granularity content"),
        }
        assert_eq!(first.font.name, "Test");
        assert_eq!(first.font.size, 12.0);
        assert!(
            first.color.is_none(),
            "black fill is elided by compact_color"
        );
        assert!(
            first.runs.is_none(),
            "a uniform-style line elides runs entirely (matches element-level font/color)"
        );

        match &compact[1] {
            CompactElement::Image(image) => {
                assert_eq!(image.bbox, [0.0, 104.0, 20.0, 106.0]);
            }
            _ => panic!("expected the image between the two text lines"),
        }

        let CompactElement::Text(third) = &compact[2] else {
            panic!("expected third (bottommost) element to be text")
        };
        match &third.content {
            CompactTextContent::Element { text } => assert_eq!(text, "foo bar"),
            CompactTextContent::Word { .. } => panic!("expected element-granularity content"),
        }
        assert!(
            third.runs.is_none(),
            "a uniform-style line elides runs entirely (matches element-level font/color)"
        );
    }

    #[test]
    fn compact_fragmented_elements_word_granularity_emits_compact_words() {
        let elements = fragmented_page_with_image();

        let compact = compact_fragmented_elements(&elements, Granularity::Word);
        assert_eq!(compact.len(), 3, "two text lines + one image");

        let CompactElement::Text(first) = &compact[0] else {
            panic!("expected first (topmost) element to be text")
        };
        let CompactTextContent::Word { words } = &first.content else {
            panic!("expected word-granularity content")
        };
        assert_eq!(words.len(), 2, "hello, world");
        assert_eq!(words[0].0, "hello");
        assert_eq!(
            (words[0].1, words[0].2, words[0].3, words[0].4),
            (0.0, 88.0, 40.0, 100.0)
        );
        assert_eq!(words[1].0, "world");
        assert_eq!(
            (words[1].1, words[1].2, words[1].3, words[1].4),
            (52.0, 88.0, 92.0, 100.0)
        );

        let CompactElement::Text(third) = &compact[2] else {
            panic!("expected third (bottommost) element to be text")
        };
        let CompactTextContent::Word { words } = &third.content else {
            panic!("expected word-granularity content")
        };
        assert_eq!(words.len(), 2, "foo, bar");
        assert_eq!(words[0].0, "foo");
        assert_eq!(
            (words[0].1, words[0].2, words[0].3, words[0].4),
            (0.0, 108.0, 24.0, 120.0)
        );
        assert_eq!(words[1].0, "bar");
        assert_eq!(
            (words[1].1, words[1].2, words[1].3, words[1].4),
            (36.0, 108.0, 60.0, 120.0)
        );
    }

    /// Two single-glyph words on one baseline, "AB" then "CD", where the
    /// first word's glyphs use `font_a` and the second's use `font_b` —
    /// i.e. a glyph-fragmented line with real intra-line style variation
    /// (unlike every other fixture in this file, which is deliberately
    /// uniform-style). Glyph width 8pt, no gap within a word, 4pt gap
    /// between words (exceeds the `0.25 * 12pt` = 3pt word-split
    /// threshold), one shared baseline so both words land on one line.
    fn make_mixed_style_line() -> Vec<Element> {
        let font_a = Font {
            name: "FontA".to_string(),
            size: 12.0,
            bold: false,
            italic: false,
        };
        let font_b = Font {
            name: "FontB".to_string(),
            size: 12.0,
            bold: true,
            italic: false,
        };
        [
            ('A', 0.0, font_a.clone()),
            ('B', 8.0, font_a),
            ('C', 20.0, font_b.clone()),
            ('D', 28.0, font_b),
        ]
        .into_iter()
        .enumerate()
        .map(|(i, (ch, x0, font))| {
            let bbox = BBox {
                x0,
                y0: 88.0,
                x1: x0 + 8.0,
                y1: 100.0,
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
                baseline_y: 100.0,
                words: vec![word],
            };
            Element::Text(TextElement {
                id: format!("m{i}"),
                bbox,
                content: ch.to_string(),
                font,
                color: color(),
                lines: Some(vec![line]),
                runs: None,
            })
        })
        .collect()
    }

    #[test]
    fn compact_fragmented_elements_mixed_style_line_produces_text_reconstructing_runs() {
        let elements = make_mixed_style_line();

        let compact = compact_fragmented_elements(&elements, Granularity::Element);
        assert_eq!(compact.len(), 1, "one physical line");

        let CompactElement::Text(text) = &compact[0] else {
            panic!("expected a text element")
        };
        let CompactTextContent::Element { text: full_text } = &text.content else {
            panic!("expected element-granularity content")
        };
        assert_eq!(full_text, "AB CD");

        let runs = text
            .runs
            .as_ref()
            .expect("intra-line style variation must produce runs");
        assert_eq!(runs.len(), 2, "run boundary at the font change");
        assert_eq!(runs[0].content, "AB ");
        assert_eq!(runs[0].font.name, "FontA");
        assert!(!runs[0].font.bold);
        assert_eq!(runs[1].content, "CD");
        assert_eq!(runs[1].font.name, "FontB");
        assert!(runs[1].font.bold);

        // The core invariant this test exists to pin: concatenating every
        // run's content in order reproduces the element's full text
        // (spaces included), matching docray-docx/docray-pptx's
        // `runs.iter().map(|r| &r.content).collect::<String>() == text`.
        let reconstructed: String = runs.iter().map(|r| r.content.as_str()).collect();
        assert_eq!(&reconstructed, full_text);
    }
}
