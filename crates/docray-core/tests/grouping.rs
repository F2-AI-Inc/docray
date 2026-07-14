use docray_core::grouping::{group_into_lines, RawChar};
use docray_model::BBox;

fn ch(c: &str, x0: f64, x1: f64, baseline: f64) -> RawChar {
    RawChar {
        content: c.into(),
        bbox: BBox {
            x0,
            y0: baseline - 10.0,
            x1,
            y1: baseline,
        },
        unicode: c.chars().next().map(|c| c as u32).unwrap_or(0),
        font_size: 10.0,
        baseline_y: baseline,
    }
}

#[test]
fn splits_words_on_whitespace_and_gaps() {
    // "Hi yo" then a big gap then "x" -> words: Hi, yo, x
    let chars = vec![
        ch("H", 0.0, 5.0, 100.0),
        ch("i", 5.0, 8.0, 100.0),
        ch(" ", 8.0, 11.0, 100.0),
        ch("y", 11.0, 16.0, 100.0),
        ch("o", 16.0, 20.0, 100.0),
        ch("x", 40.0, 45.0, 100.0), // gap 20.0 > 0.25 * 10.0
    ];
    let lines = group_into_lines(&chars);
    assert_eq!(lines.len(), 1);
    let words: Vec<&str> = lines[0].words.iter().map(|w| w.content.as_str()).collect();
    assert_eq!(words, vec!["Hi", "yo", "x"]);
    assert_eq!(lines[0].words[0].chars.len(), 2);
    assert_eq!(
        lines[0].words[0].bbox,
        BBox {
            x0: 0.0,
            y0: 90.0,
            x1: 8.0,
            y1: 100.0
        }
    );
}

#[test]
fn splits_lines_on_baseline_jump() {
    let chars = vec![
        ch("a", 0.0, 5.0, 100.0),
        ch("b", 5.0, 10.0, 100.2), // within 0.5 * font_size -> same line
        ch("c", 0.0, 5.0, 112.0),  // jump > 5.0 -> new line
    ];
    let lines = group_into_lines(&chars);
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0].baseline_y, 100.0);
    assert_eq!(lines[1].baseline_y, 112.0);
    assert_eq!(lines[1].words[0].content, "c");
}

#[test]
fn empty_and_whitespace_only_input() {
    assert!(group_into_lines(&[]).is_empty());
    let only_space = vec![ch(" ", 0.0, 3.0, 50.0)];
    let lines = group_into_lines(&only_space);
    assert!(lines.is_empty() || lines.iter().all(|l| l.words.is_empty()));
}
