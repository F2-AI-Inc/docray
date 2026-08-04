//! Page-range selection syntax: `"7"` (single page) or `"1-200"` (inclusive
//! range), both 1-based. Parsing lives here so the CLI/server error-mapping
//! layer only needs to translate one `Err(String)` into `bad_pages`.

use std::str::FromStr;

/// A 1-based, inclusive page range. `start <= end` and `start >= 1` are
/// upheld by [`FromStr`]; there is no public constructor that bypasses it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageSelection {
    pub start: u32,
    pub end: u32,
}

impl PageSelection {
    /// Number of pages covered by this selection, inclusive of both ends.
    pub fn count(&self) -> u32 {
        self.end - self.start + 1
    }
}

impl FromStr for PageSelection {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            return Err("page selection is empty".to_string());
        }

        let mut parts = trimmed.split('-');
        let first = parts
            .next()
            .ok_or_else(|| "page selection is empty".to_string())?;
        let rest: Vec<&str> = parts.collect();

        let (start, end) = match rest.len() {
            0 => {
                let page = parse_page(first)?;
                (page, page)
            }
            1 => {
                let start = parse_page(first)?;
                let end = parse_page(rest[0])?;
                (start, end)
            }
            _ => {
                return Err(format!(
                    "page selection {trimmed:?} has more than one '-' separator"
                ))
            }
        };

        if start > end {
            return Err(format!(
                "page selection {trimmed:?} is reversed: start {start} > end {end}"
            ));
        }

        Ok(PageSelection { start, end })
    }
}

fn parse_page(s: &str) -> Result<u32, String> {
    let page: u32 = s
        .parse()
        .map_err(|_| format!("{s:?} is not a valid page number"))?;
    if page == 0 {
        return Err("page numbers are 1-based; 0 is not valid".to_string());
    }
    Ok(page)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_single_page() {
        assert_eq!(
            "7".parse::<PageSelection>().unwrap(),
            PageSelection { start: 7, end: 7 }
        );
    }

    #[test]
    fn parses_range() {
        assert_eq!(
            "1-200".parse::<PageSelection>().unwrap(),
            PageSelection { start: 1, end: 200 }
        );
    }

    #[test]
    fn range_count_is_inclusive() {
        assert_eq!("1-200".parse::<PageSelection>().unwrap().count(), 200);
    }

    #[test]
    fn rejects_empty() {
        assert!("".parse::<PageSelection>().is_err());
    }

    #[test]
    fn rejects_non_numeric() {
        assert!("abc".parse::<PageSelection>().is_err());
    }

    #[test]
    fn rejects_zero_page() {
        assert!("0".parse::<PageSelection>().is_err());
    }

    #[test]
    fn rejects_zero_in_range() {
        assert!("0-5".parse::<PageSelection>().is_err());
    }

    #[test]
    fn rejects_reversed_range() {
        assert!("200-1".parse::<PageSelection>().is_err());
    }

    #[test]
    fn rejects_multiple_separators() {
        assert!("1-2-3".parse::<PageSelection>().is_err());
    }

    #[test]
    fn rejects_leading_dash() {
        assert!("-5".parse::<PageSelection>().is_err());
    }

    #[test]
    fn single_page_range_is_valid() {
        assert_eq!(
            "5-5".parse::<PageSelection>().unwrap(),
            PageSelection { start: 5, end: 5 }
        );
    }
}
