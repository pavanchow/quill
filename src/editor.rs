//! The editor ties the piece table, cursors, search, and an undo/redo history
//! into one editable document.
//!
//! Every content change flows through `insert`, `delete`, or `replace`, which
//! record the inverse information needed to undo the change. Changes are grouped
//! into transactions. Consecutive single character inserts at the caret coalesce
//! into one transaction so that undo removes a whole typed word rather than one
//! letter at a time; any delete, any non contiguous edit, or an explicit
//! `commit` starts a fresh transaction. Making a new edit clears the redo stack,
//! so redo is only ever valid until the next change.

use crate::cursor::{Cursor, CursorSet};
use crate::piece_table::{PieceInfo, PieceTable};
use crate::search;

/// A primitive, invertible change to the buffer.
#[derive(Clone, Debug)]
enum Op {
    /// `text` was inserted starting at character offset `at`.
    Insert { at: usize, text: String },
    /// `text` was removed starting at character offset `at`.
    Delete { at: usize, text: String },
}

/// A group of primitive ops that undo and redo as a unit.
#[derive(Clone, Debug, Default)]
struct Transaction {
    ops: Vec<Op>,
    /// True while the transaction may still absorb contiguous typing.
    coalescing: bool,
    /// Character offset just past the last coalesced insert.
    insert_end: usize,
}

/// An editable text document.
#[derive(Clone)]
pub struct TextEditor {
    buffer: PieceTable,
    cursors: CursorSet,
    undo: Vec<Transaction>,
    redo: Vec<Transaction>,
    pending: Transaction,
}

impl TextEditor {
    /// Open a document from initial text.
    pub fn new(text: &str) -> Self {
        TextEditor {
            buffer: PieceTable::new(text),
            cursors: CursorSet::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            pending: Transaction::default(),
        }
    }

    /// An empty document.
    pub fn empty() -> Self {
        TextEditor::new("")
    }

    /// Read only access to the underlying buffer.
    pub fn buffer(&self) -> &PieceTable {
        &self.buffer
    }

    /// The current cursor set.
    pub fn cursors(&self) -> &CursorSet {
        &self.cursors
    }

    /// Replace the cursor set.
    pub fn set_cursors(&mut self, cursors: CursorSet) {
        self.cursors = cursors;
        self.cursors.clamp(self.buffer.char_len());
    }

    /// Full document contents.
    pub fn contents(&self) -> String {
        self.buffer.contents()
    }

    /// Number of characters.
    pub fn char_len(&self) -> usize {
        self.buffer.char_len()
    }

    /// Whether the document holds no characters.
    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    /// Number of lines.
    pub fn line_count(&self) -> usize {
        self.buffer.line_count()
    }

    /// Zero based line and column for a character offset.
    pub fn line_col(&self, pos: usize) -> (usize, usize) {
        self.buffer.offset_to_line_col(pos)
    }

    /// The text of a zero based line, without its newline.
    pub fn line(&self, line: usize) -> String {
        self.buffer.line_slice(line)
    }

    /// Character offsets of every match of `needle`.
    pub fn find_all(&self, needle: &str) -> Vec<usize> {
        search::find_all(&self.buffer.contents(), needle)
    }

    /// A structural view of the pieces backing the buffer.
    pub fn pieces(&self) -> Vec<PieceInfo> {
        self.buffer.piece_view()
    }

    /// End the current transaction so the next edit starts a new undo group.
    pub fn commit(&mut self) {
        if !self.pending.ops.is_empty() {
            let done = std::mem::take(&mut self.pending);
            self.undo.push(done);
        }
        self.pending.coalescing = false;
    }

    fn push_op(&mut self, op: Op, coalescable_insert_at: Option<usize>) {
        self.redo.clear();
        match coalescable_insert_at {
            Some(at)
                if self.pending.coalescing
                    && !self.pending.ops.is_empty()
                    && self.pending.insert_end == at =>
            {
                // Extend the trailing insert of the current transaction.
                if let Some(Op::Insert { text, .. }) = self.pending.ops.last_mut() {
                    if let Op::Insert { text: new_text, .. } = &op {
                        text.push_str(new_text);
                        self.pending.insert_end = at + new_text.chars().count();
                        return;
                    }
                }
                self.pending.ops.push(op);
            }
            _ => {
                self.commit();
                if let Op::Insert { at, ref text } = op {
                    self.pending.coalescing = true;
                    self.pending.insert_end = at + text.chars().count();
                } else {
                    self.pending.coalescing = false;
                }
                self.pending.ops.push(op);
            }
        }
    }

    /// Record `ops` as one committed transaction, dropping any redo history.
    fn push_tx(&mut self, ops: Vec<Op>) {
        if ops.is_empty() {
            return;
        }
        self.commit();
        self.redo.clear();
        self.pending.ops = ops;
        self.pending.coalescing = false;
        self.commit();
    }

    /// Insert `text` at character offset `pos`, starting a new undo group.
    pub fn insert(&mut self, pos: usize, text: &str) {
        if text.is_empty() {
            return;
        }
        self.buffer.insert(pos, text);
        self.commit();
        self.pending.coalescing = false;
        self.push_op(
            Op::Insert {
                at: pos,
                text: text.to_string(),
            },
            None,
        );
        self.commit();
        self.cursors.clamp(self.buffer.char_len());
    }

    /// Type `text` at character offset `pos`, coalescing with adjacent typing.
    pub fn type_text(&mut self, pos: usize, text: &str) {
        if text.is_empty() {
            return;
        }
        self.buffer.insert(pos, text);
        self.push_op(
            Op::Insert {
                at: pos,
                text: text.to_string(),
            },
            Some(pos),
        );
        self.cursors.clamp(self.buffer.char_len());
    }

    /// Delete the characters in `start..end`, starting a new undo group.
    pub fn delete(&mut self, start: usize, end: usize) {
        if start == end {
            return;
        }
        let removed = self.buffer.slice(start, end);
        self.buffer.delete(start, end);
        self.push_op(
            Op::Delete {
                at: start,
                text: removed,
            },
            None,
        );
        self.commit();
        self.cursors.clamp(self.buffer.char_len());
    }

    /// Replace `start..end` with `text` as a single undo group.
    pub fn replace(&mut self, start: usize, end: usize, text: &str) {
        self.commit();
        if start != end {
            let removed = self.buffer.slice(start, end);
            self.buffer.delete(start, end);
            self.pending.ops.push(Op::Delete {
                at: start,
                text: removed,
            });
        }
        if !text.is_empty() {
            self.buffer.insert(start, text);
            self.pending.ops.push(Op::Insert {
                at: start,
                text: text.to_string(),
            });
        }
        self.pending.coalescing = false;
        self.redo.clear();
        self.commit();
        self.cursors.clamp(self.buffer.char_len());
    }

    fn apply_inverse(&mut self, tx: &Transaction) {
        // Undo by inverting each op in reverse order.
        for op in tx.ops.iter().rev() {
            match op {
                Op::Insert { at, text } => {
                    let n = text.chars().count();
                    self.buffer.delete(*at, at + n);
                }
                Op::Delete { at, text } => {
                    self.buffer.insert(*at, text);
                }
            }
        }
    }

    fn apply_forward(&mut self, tx: &Transaction) {
        for op in &tx.ops {
            match op {
                Op::Insert { at, text } => {
                    self.buffer.insert(*at, text);
                }
                Op::Delete { at, text } => {
                    let n = text.chars().count();
                    self.buffer.delete(*at, at + n);
                }
            }
        }
    }

    /// Undo the most recent transaction. Returns false when nothing is left.
    pub fn undo(&mut self) -> bool {
        self.commit();
        if let Some(tx) = self.undo.pop() {
            self.apply_inverse(&tx);
            self.redo.push(tx);
            self.cursors.clamp(self.buffer.char_len());
            true
        } else {
            false
        }
    }

    /// Redo the most recently undone transaction. Returns false when none.
    pub fn redo(&mut self) -> bool {
        self.commit();
        if let Some(tx) = self.redo.pop() {
            self.apply_forward(&tx);
            self.undo.push(tx.clone());
            self.cursors.clamp(self.buffer.char_len());
            true
        } else {
            false
        }
    }

    /// Number of committed undo transactions on the stack. Useful for tests
    /// that need to know whether an operation actually recorded a change.
    pub fn history_len(&self) -> usize {
        self.undo.len()
    }

    /// Whether an undo is available.
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty() || !self.pending.ops.is_empty()
    }

    /// Whether a redo is available.
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Insert `text` at every cursor, replacing each cursor's selection when
    /// one is active. Cursors are applied from the highest offset down so
    /// lower offsets stay valid, and each cursor's edit is recorded as its own
    /// undo transaction: undo walks back one cursor at a time. Returns the
    /// number of cursors that edited. Every caret ends up just past its own
    /// copy of `text`.
    ///
    /// Selections are expected not to overlap; overlapping selections are
    /// still applied deterministically in descending start order.
    pub fn insert_at_cursors(&mut self, text: &str) -> usize {
        if text.is_empty() || self.cursors.is_empty() {
            return 0;
        }
        let n = text.chars().count();
        // Edit start per cursor (selection start when a selection is active),
        // in descending order so lower offsets stay valid while applying.
        let mut edits: Vec<(usize, Option<(usize, usize)>)> = self
            .cursors
            .cursors()
            .iter()
            .map(|c| (c.caret, c.selected_range()))
            .map(|(caret, sel)| match sel {
                Some((s, e)) => (s, Some((s, e))),
                None => (caret, None),
            })
            .collect();
        edits.sort_by_key(|&(at, _)| std::cmp::Reverse(at));

        for &(at, sel) in &edits {
            let mut ops = Vec::with_capacity(2);
            if let Some((s, e)) = sel {
                let removed = self.buffer.slice(s, e);
                self.buffer.delete(s, e);
                ops.push(Op::Delete { at: s, text: removed });
            }
            self.buffer.insert(at, text);
            ops.push(Op::Insert {
                at,
                text: text.to_string(),
            });
            self.push_tx(ops);
        }

        // Recompute carets: each sits just past its inserted copy, shifted by
        // every lower edit's insertion. Walk ascending with a running shift.
        let applied = edits.len();
        let mut ascending = edits;
        ascending.sort_by_key(|&(at, _)| at);
        let mut shift = 0usize;
        let mut new_cursors = Vec::with_capacity(ascending.len());
        for &(at, _) in &ascending {
            new_cursors.push(Cursor::at(at + n + shift));
            shift += n;
        }
        self.set_cursors(CursorSet::from_cursors(new_cursors));
        applied
    }

    /// Delete every cursor's selected range, highest offset first, one undo
    /// transaction per deletion. Returns the number of cursors that deleted
    /// something. Cursors that deleted collapse to their selection starts;
    /// bare carets stay put. Both are shifted by lower deletions.
    pub fn delete_selections(&mut self) -> usize {
        let mut deletions: Vec<(usize, usize)> = self
            .cursors
            .cursors()
            .iter()
            .filter_map(|c| c.selected_range())
            .collect();
        if deletions.is_empty() {
            return 0;
        }
        deletions.sort_by_key(|&(s, _)| std::cmp::Reverse(s));
        let mut count = 0usize;
        for (s, e) in &deletions {
            let removed = self.buffer.slice(*s, *e);
            self.buffer.delete(*s, *e);
            self.push_tx(vec![Op::Delete {
                at: *s,
                text: removed,
            }]);
            count += 1;
        }
        // Rebuild the cursor set: deleted selections collapse to their start,
        // bare carets stay, everything shifts by lower deletions. At equal
        // offsets a bare caret is processed before a deletion starting there
        // so it is not shifted by that deletion.
        let mut records: Vec<(usize, Option<(usize, usize)>)> = self
            .cursors
            .cursors()
            .iter()
            .map(|c| (c.caret, c.selected_range()))
            .map(|(caret, sel)| match sel {
                Some((s, e)) => (s, Some((s, e))),
                None => (caret, None),
            })
            .collect();
        records.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.is_none().cmp(&a.1.is_none())));
        let mut shift = 0isize;
        let mut new_cursors = Vec::with_capacity(records.len());
        for &(point, sel) in &records {
            match sel {
                Some((s, e)) => {
                    new_cursors.push(Cursor::at((point as isize + shift) as usize));
                    shift -= (e - s) as isize;
                }
                None => new_cursors.push(Cursor::at((point as isize + shift) as usize)),
            }
        }
        self.set_cursors(CursorSet::from_cursors(new_cursors));
        count
    }

    /// Replace every non-overlapping occurrence of `needle` with `replacement`
    /// as a single undo group. Occurrences are found left to right on the
    /// current text and applied from the highest offset down, so an
    /// `replacement` containing `needle` is never rescanned. Returns the
    /// number of replacements. An empty needle replaces nothing.
    pub fn replace_all(&mut self, needle: &str, replacement: &str) -> usize {
        if needle.is_empty() {
            return 0;
        }
        let contents = self.buffer.contents();
        let matches = search::find_all(&contents, needle);
        if matches.is_empty() {
            return 0;
        }
        let n = needle.chars().count();
        let mut ops = Vec::with_capacity(matches.len() * 2);
        for &start in matches.iter().rev() {
            let end = start + n;
            let removed = self.buffer.slice(start, end);
            self.buffer.delete(start, end);
            ops.push(Op::Delete {
                at: start,
                text: removed,
            });
            if !replacement.is_empty() {
                self.buffer.insert(start, replacement);
                ops.push(Op::Insert {
                    at: start,
                    text: replacement.to_string(),
                });
            }
        }
        self.push_tx(ops);
        self.cursors.clamp(self.buffer.char_len());
        matches.len()
    }

    /// Interactive replace: replace the first occurrence of `needle` at or
    /// after character offset `from` with `replacement` as one undo group.
    /// Returns the range covering the inserted replacement so the caller can
    /// continue searching past it, or `None` when nothing matches.
    pub fn replace_next(
        &mut self,
        needle: &str,
        replacement: &str,
        from: usize,
    ) -> Option<(usize, usize)> {
        if needle.is_empty() || from > self.buffer.char_len() {
            return None;
        }
        let contents = self.buffer.contents();
        let hits = search::find_all(&contents, needle);
        let &start = hits.iter().find(|&&h| h >= from)?;
        let end = start + needle.chars().count();
        let removed = self.buffer.slice(start, end);
        self.buffer.delete(start, end);
        let mut ops = vec![Op::Delete {
            at: start,
            text: removed,
        }];
        if !replacement.is_empty() {
            self.buffer.insert(start, replacement);
            ops.push(Op::Insert {
                at: start,
                text: replacement.to_string(),
            });
        }
        self.push_tx(ops);
        self.cursors.clamp(self.buffer.char_len());
        Some((start, start + replacement.chars().count()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_delete_undo_redo() {
        let mut ed = TextEditor::new("hello");
        ed.insert(5, " world");
        assert_eq!(ed.contents(), "hello world");
        ed.delete(0, 6);
        assert_eq!(ed.contents(), "world");
        assert!(ed.undo());
        assert_eq!(ed.contents(), "hello world");
        assert!(ed.undo());
        assert_eq!(ed.contents(), "hello");
        assert!(!ed.undo());
        assert!(ed.redo());
        assert_eq!(ed.contents(), "hello world");
        assert!(ed.redo());
        assert_eq!(ed.contents(), "world");
    }

    #[test]
    fn typing_coalesces_into_one_undo() {
        let mut ed = TextEditor::empty();
        for (i, c) in "hello".chars().enumerate() {
            ed.type_text(i, &c.to_string());
        }
        assert_eq!(ed.contents(), "hello");
        assert!(ed.undo());
        assert_eq!(ed.contents(), "");
        assert!(ed.redo());
        assert_eq!(ed.contents(), "hello");
    }

    #[test]
    fn new_edit_invalidates_redo() {
        let mut ed = TextEditor::new("abc");
        ed.delete(0, 1);
        assert!(ed.undo());
        assert_eq!(ed.contents(), "abc");
        ed.insert(3, "d");
        assert!(!ed.can_redo());
        assert_eq!(ed.contents(), "abcd");
    }

    #[test]
    fn replace_is_single_group() {
        let mut ed = TextEditor::new("the cat sat");
        ed.replace(4, 7, "dog");
        assert_eq!(ed.contents(), "the dog sat");
        assert!(ed.undo());
        assert_eq!(ed.contents(), "the cat sat");
    }
}
