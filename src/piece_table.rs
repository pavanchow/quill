//! Piece table text buffer.
//!
//! The buffer keeps two immutable character stores: `original` (the text the
//! buffer was opened with) and `add` (an append only store that every insert
//! writes into). The visible document is a sequence of `Piece` values, each of
//! which points at a byte range inside one of the two stores. Editing never
//! rewrites large regions of text: an insert appends to `add` and splices a new
//! piece into the list, and a delete trims or drops pieces. That gives cheap
//! edits on large documents because the cost is proportional to the number of
//! pieces touched, not the size of the text.
//!
//! All public offsets are character offsets (counts of Unicode scalar values),
//! not byte offsets. A character offset can never land inside a multibyte UTF-8
//! sequence, so edits at any offset are always valid and never panic on a
//! boundary. Byte lengths are still available through `byte_len` for callers
//! that need them.

/// Which backing store a piece reads from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Source {
    Original,
    Add,
}

/// A contiguous run of text drawn from one backing store.
#[derive(Clone, Debug)]
struct Piece {
    source: Source,
    /// Byte offset of the run inside its backing store.
    start: usize,
    /// Byte length of the run.
    byte_len: usize,
    /// Number of characters in the run.
    char_len: usize,
    /// Number of `\n` bytes in the run.
    newlines: usize,
    /// Characters after the run's last `\n`. Equals `char_len` when the run has
    /// no newline. Used to advance column state when skipping a whole piece.
    tail_chars: usize,
}

/// Character count and newline metadata for a slice of text.
fn measure(s: &str) -> (usize, usize, usize) {
    let mut chars = 0;
    let mut newlines = 0;
    let mut tail = 0;
    for c in s.chars() {
        chars += 1;
        if c == '\n' {
            newlines += 1;
            tail = 0;
        } else {
            tail += 1;
        }
    }
    (chars, newlines, tail)
}

/// Byte offset of the `k`th character boundary in `s` (`k` in `0..=char count`).
fn char_to_byte_index(s: &str, k: usize) -> usize {
    if k == 0 {
        return 0;
    }
    for (count, (byte, _)) in s.char_indices().enumerate() {
        if count == k {
            return byte;
        }
    }
    s.len()
}

/// A piece table backed text buffer.
#[derive(Clone)]
pub struct PieceTable {
    original: String,
    add: String,
    pieces: Vec<Piece>,
    char_len: usize,
    newline_total: usize,
}

impl PieceTable {
    /// Build a buffer from initial text.
    pub fn new(text: &str) -> Self {
        let mut pt = PieceTable {
            original: text.to_string(),
            add: String::new(),
            pieces: Vec::new(),
            char_len: 0,
            newline_total: 0,
        };
        if !text.is_empty() {
            let (chars, newlines, tail) = measure(text);
            pt.pieces.push(Piece {
                source: Source::Original,
                start: 0,
                byte_len: text.len(),
                char_len: chars,
                newlines,
                tail_chars: tail,
            });
            pt.char_len = chars;
            pt.newline_total = newlines;
        }
        pt
    }

    /// An empty buffer.
    pub fn empty() -> Self {
        PieceTable::new("")
    }

    /// Total number of characters in the buffer.
    pub fn char_len(&self) -> usize {
        self.char_len
    }

    /// Total number of bytes in the buffer.
    pub fn byte_len(&self) -> usize {
        self.pieces.iter().map(|p| p.byte_len).sum()
    }

    /// Whether the buffer holds no characters.
    pub fn is_empty(&self) -> bool {
        self.char_len == 0
    }

    /// Number of lines. An empty buffer has one line. A trailing newline creates
    /// a final empty line.
    pub fn line_count(&self) -> usize {
        self.newline_total + 1
    }

    fn store(&self, source: Source) -> &str {
        match source {
            Source::Original => &self.original,
            Source::Add => &self.add,
        }
    }

    fn piece_slice(&self, p: &Piece) -> &str {
        &self.store(p.source)[p.start..p.start + p.byte_len]
    }

    /// Ensure a piece boundary exists at character offset `pos`, returning the
    /// index in `pieces` at which the boundary sits (a piece is split if `pos`
    /// falls inside one). `pos` must be in `0..=char_len`.
    fn split_at(&mut self, pos: usize) -> usize {
        assert!(pos <= self.char_len, "offset out of range");
        if pos == 0 {
            return 0;
        }
        let mut acc = 0;
        for i in 0..self.pieces.len() {
            let c = self.pieces[i].char_len;
            if acc + c == pos {
                return i + 1;
            }
            if acc + c > pos {
                let k = pos - acc;
                let piece = self.pieces[i].clone();
                let slice = self.piece_slice(&piece);
                let byte_k = char_to_byte_index(slice, k);
                let (lchars, lnl, ltail) = measure(&slice[..byte_k]);
                let (rchars, rnl, rtail) = measure(&slice[byte_k..]);
                let left = Piece {
                    source: piece.source,
                    start: piece.start,
                    byte_len: byte_k,
                    char_len: lchars,
                    newlines: lnl,
                    tail_chars: ltail,
                };
                let right = Piece {
                    source: piece.source,
                    start: piece.start + byte_k,
                    byte_len: piece.byte_len - byte_k,
                    char_len: rchars,
                    newlines: rnl,
                    tail_chars: rtail,
                };
                self.pieces[i] = left;
                self.pieces.insert(i + 1, right);
                return i + 1;
            }
            acc += c;
        }
        self.pieces.len()
    }

    /// Insert `text` at character offset `pos`.
    pub fn insert(&mut self, pos: usize, text: &str) {
        assert!(pos <= self.char_len, "insert offset out of range");
        if text.is_empty() {
            return;
        }
        let add_start = self.add.len();
        self.add.push_str(text);
        let (chars, newlines, tail) = measure(text);
        let new_piece = Piece {
            source: Source::Add,
            start: add_start,
            byte_len: text.len(),
            char_len: chars,
            newlines,
            tail_chars: tail,
        };
        // Coalesce a contiguous typing run: if inserting exactly at the end of
        // the last piece and that piece is the tail of the add store, just grow
        // it instead of creating a new piece.
        if pos == self.char_len {
            if let Some(last) = self.pieces.last_mut() {
                if last.source == Source::Add && last.start + last.byte_len == add_start {
                    last.byte_len += text.len();
                    last.char_len += chars;
                    last.newlines += newlines;
                    last.tail_chars = if newlines > 0 {
                        tail
                    } else {
                        last.tail_chars + tail
                    };
                    self.char_len += chars;
                    self.newline_total += newlines;
                    return;
                }
            }
        }
        let idx = self.split_at(pos);
        self.pieces.insert(idx, new_piece);
        self.char_len += chars;
        self.newline_total += newlines;
    }

    /// Delete the characters in `start..end`.
    pub fn delete(&mut self, start: usize, end: usize) {
        assert!(start <= end, "delete range reversed");
        assert!(end <= self.char_len, "delete range out of bounds");
        if start == end {
            return;
        }
        let i = self.split_at(start);
        let j = self.split_at(end);
        for p in self.pieces.drain(i..j) {
            self.char_len -= p.char_len;
            self.newline_total -= p.newlines;
        }
    }

    /// Replace the characters in `start..end` with `text`.
    pub fn replace(&mut self, start: usize, end: usize, text: &str) {
        self.delete(start, end);
        self.insert(start, text);
    }

    /// Collect the characters in `start..end` into a `String`.
    pub fn slice(&self, start: usize, end: usize) -> String {
        assert!(start <= end, "slice range reversed");
        assert!(end <= self.char_len, "slice range out of bounds");
        let mut out = String::new();
        if start == end {
            return out;
        }
        let mut acc = 0;
        for p in &self.pieces {
            let p_start = acc;
            let p_end = acc + p.char_len;
            if p_end <= start {
                acc = p_end;
                continue;
            }
            if p_start >= end {
                break;
            }
            let slice = self.piece_slice(p);
            let from = start.saturating_sub(p_start);
            let to = if end < p_end { end - p_start } else { p.char_len };
            let bfrom = char_to_byte_index(slice, from);
            let bto = char_to_byte_index(slice, to);
            out.push_str(&slice[bfrom..bto]);
            acc = p_end;
        }
        out
    }

    /// The full buffer contents as a `String`.
    pub fn contents(&self) -> String {
        let mut out = String::with_capacity(self.byte_len());
        for p in &self.pieces {
            out.push_str(self.piece_slice(p));
        }
        out
    }

    /// Map a character offset to a zero based `(line, column)` pair.
    pub fn offset_to_line_col(&self, pos: usize) -> (usize, usize) {
        assert!(pos <= self.char_len, "offset out of range");
        let mut line = 0;
        let mut col = 0;
        let mut seen = 0;
        for p in &self.pieces {
            if seen + p.char_len <= pos {
                if p.newlines > 0 {
                    line += p.newlines;
                    col = p.tail_chars;
                } else {
                    col += p.char_len;
                }
                seen += p.char_len;
                if seen == pos {
                    return (line, col);
                }
            } else {
                let k = pos - seen;
                let slice = self.piece_slice(p);
                for c in slice.chars().take(k) {
                    if c == '\n' {
                        line += 1;
                        col = 0;
                    } else {
                        col += 1;
                    }
                }
                return (line, col);
            }
        }
        (line, col)
    }

    /// Character offset at which `line` begins (zero based line index). A line
    /// past the end clamps to the buffer length.
    pub fn line_start_offset(&self, line: usize) -> usize {
        if line == 0 {
            return 0;
        }
        let mut seen_lines = 0;
        let mut off = 0;
        for p in &self.pieces {
            if seen_lines + p.newlines >= line {
                let slice = self.piece_slice(p);
                let mut local = 0;
                for c in slice.chars() {
                    local += 1;
                    if c == '\n' {
                        seen_lines += 1;
                        if seen_lines == line {
                            return off + local;
                        }
                    }
                }
                off += p.char_len;
            } else {
                seen_lines += p.newlines;
                off += p.char_len;
            }
        }
        off
    }

    /// Map a zero based `(line, column)` pair to a character offset, clamped to
    /// the end of that line and to the buffer.
    pub fn line_col_to_offset(&self, line: usize, col: usize) -> usize {
        let start = self.line_start_offset(line);
        let end = if line + 1 < self.line_count() {
            // One before the next line start drops the newline character.
            self.line_start_offset(line + 1) - 1
        } else {
            self.char_len
        };
        (start + col).min(end)
    }

    /// The text of `line` (zero based), without its trailing newline.
    pub fn line_slice(&self, line: usize) -> String {
        let start = self.line_start_offset(line);
        let end = if line + 1 < self.line_count() {
            self.line_start_offset(line + 1) - 1
        } else {
            self.char_len
        };
        self.slice(start, end)
    }

    /// Number of pieces currently in the table (a rough structural size).
    pub fn piece_count(&self) -> usize {
        self.pieces.len()
    }

    /// A read only view of the pieces for visualization and tests.
    pub fn piece_view(&self) -> Vec<PieceInfo> {
        self.pieces
            .iter()
            .map(|p| PieceInfo {
                source: match p.source {
                    Source::Original => "original",
                    Source::Add => "add",
                },
                start: p.start,
                byte_len: p.byte_len,
                char_len: p.char_len,
                text: self.piece_slice(p).to_string(),
            })
            .collect()
    }
}

/// A snapshot of a single piece, used for visualization.
#[derive(Clone, Debug)]
pub struct PieceInfo {
    pub source: &'static str,
    pub start: usize,
    pub byte_len: usize,
    pub char_len: usize,
    pub text: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ref_line_col(s: &str, pos: usize) -> (usize, usize) {
        let mut line = 0;
        let mut col = 0;
        for c in s.chars().take(pos) {
            if c == '\n' {
                line += 1;
                col = 0;
            } else {
                col += 1;
            }
        }
        (line, col)
    }

    #[test]
    fn insert_and_contents() {
        let mut pt = PieceTable::new("hello world");
        pt.insert(5, ",");
        pt.insert(pt.char_len(), "!");
        assert_eq!(pt.contents(), "hello, world!");
        assert_eq!(pt.char_len(), 13);
    }

    #[test]
    fn delete_and_replace() {
        let mut pt = PieceTable::new("hello world");
        pt.delete(0, 6);
        assert_eq!(pt.contents(), "world");
        pt.replace(0, 5, "there");
        assert_eq!(pt.contents(), "there");
    }

    #[test]
    fn multibyte_safe() {
        let mut pt = PieceTable::new("café résumé");
        assert_eq!(pt.char_len(), 11);
        pt.insert(4, "X");
        assert_eq!(pt.contents(), "caféX résumé");
        pt.delete(0, 5);
        assert_eq!(pt.contents(), " résumé");
    }

    #[test]
    fn line_queries() {
        let pt = PieceTable::new("ab\ncde\n\nfg");
        assert_eq!(pt.line_count(), 4);
        assert_eq!(pt.line_slice(0), "ab");
        assert_eq!(pt.line_slice(1), "cde");
        assert_eq!(pt.line_slice(2), "");
        assert_eq!(pt.line_slice(3), "fg");
        for pos in 0..=pt.char_len() {
            assert_eq!(pt.offset_to_line_col(pos), ref_line_col("ab\ncde\n\nfg", pos));
        }
    }

    #[test]
    fn line_col_round_trip() {
        let pt = PieceTable::new("ab\ncde\n\nfg");
        for line in 0..pt.line_count() {
            let start = pt.line_start_offset(line);
            let (l, c) = pt.offset_to_line_col(start);
            assert_eq!((l, c), (line, 0));
            assert_eq!(pt.line_col_to_offset(l, c), start);
        }
    }

    #[test]
    fn boundary_edits_do_not_panic() {
        let mut pt = PieceTable::empty();
        pt.insert(0, "");
        pt.delete(0, 0);
        pt.insert(0, "x");
        pt.delete(0, 1);
        assert!(pt.is_empty());
        assert_eq!(pt.line_count(), 1);
        pt.insert(0, "end");
        pt.insert(3, "!");
        assert_eq!(pt.contents(), "end!");
    }

    #[test]
    fn trailing_newline_line_count() {
        let pt = PieceTable::new("a\n");
        assert_eq!(pt.line_count(), 2);
        assert_eq!(pt.line_slice(1), "");
    }
}
