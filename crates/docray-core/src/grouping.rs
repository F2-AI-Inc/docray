use docray_model::{BBox, Char, Line, Word};

pub struct RawChar {
    pub content: String,
    pub bbox: BBox,
    pub unicode: u32,
    pub font_size: f64,
    pub baseline_y: f64,
}

fn is_whitespace(c: &RawChar) -> bool {
    c.content
        .chars()
        .next()
        .map(char::is_whitespace)
        .unwrap_or(true)
}

pub fn group_into_lines(chars: &[RawChar]) -> Vec<Line> {
    let mut lines: Vec<Line> = Vec::new();
    let mut current: Vec<&RawChar> = Vec::new();
    let mut first_baseline = 0.0_f64;

    let flush = |members: &[&RawChar], lines: &mut Vec<Line>| {
        if let Some(line) = build_line(members) {
            lines.push(line);
        }
    };

    for c in chars {
        if current.is_empty() {
            first_baseline = c.baseline_y;
            current.push(c);
            continue;
        }
        if (c.baseline_y - first_baseline).abs() > 0.5 * c.font_size {
            flush(&current, &mut lines);
            current.clear();
            first_baseline = c.baseline_y;
        }
        current.push(c);
    }
    flush(&current, &mut lines);
    lines
}

fn build_line(members: &[&RawChar]) -> Option<Line> {
    let mut words: Vec<Word> = Vec::new();
    let mut word_chars: Vec<&RawChar> = Vec::new();
    let mut prev_visible: Option<&RawChar> = None;

    let flush_word = |chars: &[&RawChar], words: &mut Vec<Word>| {
        if chars.is_empty() {
            return;
        }
        let bbox = chars
            .iter()
            .skip(1)
            .fold(chars[0].bbox, |acc, c| acc.union(&c.bbox));
        words.push(Word {
            content: chars.iter().map(|c| c.content.as_str()).collect(),
            bbox,
            chars: chars
                .iter()
                .map(|c| Char {
                    content: c.content.clone(),
                    bbox: c.bbox,
                    unicode: c.unicode,
                })
                .collect(),
        });
    };

    for c in members {
        if is_whitespace(c) {
            flush_word(&word_chars, &mut words);
            word_chars.clear();
            prev_visible = None;
            continue;
        }
        if let Some(prev) = prev_visible {
            if c.bbox.x0 - prev.bbox.x1 > 0.25 * c.font_size {
                flush_word(&word_chars, &mut words);
                word_chars.clear();
            }
        }
        word_chars.push(c);
        prev_visible = Some(c);
    }
    flush_word(&word_chars, &mut words);

    if words.is_empty() {
        return None;
    }
    let bbox = words
        .iter()
        .skip(1)
        .fold(words[0].bbox, |acc, w| acc.union(&w.bbox));
    Some(Line {
        bbox,
        baseline_y: members[0].baseline_y,
        words,
    })
}
