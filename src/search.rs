//! Substring search over buffer contents.
//!
//! Searching works on a materialized snapshot of the buffer text so that match
//! positions can be reported as character offsets, which is what every other
//! part of the API speaks. Matches are non overlapping and returned left to
//! right.

/// Character offsets of every non overlapping occurrence of `needle` in
/// `haystack`. An empty needle yields no matches.
pub fn find_all(haystack: &str, needle: &str) -> Vec<usize> {
    let mut out = Vec::new();
    if needle.is_empty() {
        return out;
    }
    let mut byte = 0;
    // Precompute a byte offset to char offset map lazily as we advance.
    let mut char_at = CharCounter::new(haystack);
    while let Some(rel) = haystack[byte..].find(needle) {
        let abs = byte + rel;
        out.push(char_at.chars_before(abs));
        byte = abs + needle.len();
    }
    out
}

/// Number of matches of `needle` in `haystack`.
pub fn count(haystack: &str, needle: &str) -> usize {
    find_all(haystack, needle).len()
}

/// Turns byte offsets into character offsets while only ever moving forward.
struct CharCounter<'a> {
    s: &'a str,
    last_byte: usize,
    last_char: usize,
}

impl<'a> CharCounter<'a> {
    fn new(s: &'a str) -> Self {
        CharCounter {
            s,
            last_byte: 0,
            last_char: 0,
        }
    }

    fn chars_before(&mut self, byte: usize) -> usize {
        let extra = self.s[self.last_byte..byte].chars().count();
        self.last_char += extra;
        self.last_byte = byte;
        self.last_char
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_matches() {
        assert_eq!(find_all("abcabcabc", "abc"), vec![0, 3, 6]);
        assert_eq!(find_all("aaaa", "aa"), vec![0, 2]);
        assert_eq!(find_all("hello", "z"), Vec::<usize>::new());
        assert_eq!(find_all("hello", ""), Vec::<usize>::new());
    }

    #[test]
    fn multibyte_offsets_are_char_offsets() {
        // "é" is two bytes; matches after it must still report char offsets.
        let hay = "éXéXé";
        assert_eq!(find_all(hay, "X"), vec![1, 3]);
    }
}
