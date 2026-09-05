//! Cursor and selection model.
//!
//! A cursor is a caret position plus an optional selection anchor, both measured
//! in character offsets. When an anchor is set the selection covers the range
//! between the anchor and the caret in either direction. Several cursors can be
//! held at once for multiple cursor editing; positions are kept sorted and
//! de-duplicated.

/// A single caret with an optional selection anchor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cursor {
    /// Caret position as a character offset.
    pub caret: usize,
    /// Selection anchor as a character offset, if a selection is active.
    pub anchor: Option<usize>,
}

impl Cursor {
    /// A cursor at `pos` with no selection.
    pub fn at(pos: usize) -> Self {
        Cursor {
            caret: pos,
            anchor: None,
        }
    }

    /// A cursor selecting `start..end` with the caret at `end`.
    pub fn selection(start: usize, end: usize) -> Self {
        Cursor {
            caret: end,
            anchor: Some(start),
        }
    }

    /// Whether a selection is active and non empty.
    pub fn has_selection(&self) -> bool {
        matches!(self.anchor, Some(a) if a != self.caret)
    }

    /// The selected range as `(low, high)`, or `None` when empty.
    pub fn selected_range(&self) -> Option<(usize, usize)> {
        match self.anchor {
            Some(a) if a != self.caret => Some((a.min(self.caret), a.max(self.caret))),
            _ => None,
        }
    }

    /// Move the caret to `pos`, dropping any selection.
    pub fn move_to(&mut self, pos: usize) {
        self.caret = pos;
        self.anchor = None;
    }

    /// Extend the selection so the caret moves to `pos`, keeping the anchor.
    pub fn extend_to(&mut self, pos: usize) {
        if self.anchor.is_none() {
            self.anchor = Some(self.caret);
        }
        self.caret = pos;
    }

    /// Drop any selection, leaving the caret in place.
    pub fn clear_selection(&mut self) {
        self.anchor = None;
    }
}

/// An ordered set of cursors for multiple cursor editing.
#[derive(Clone, Debug, Default)]
pub struct CursorSet {
    cursors: Vec<Cursor>,
}

impl CursorSet {
    /// A set with a single cursor at the origin.
    pub fn new() -> Self {
        CursorSet {
            cursors: vec![Cursor::at(0)],
        }
    }

    /// A set built from explicit cursors, normalized.
    pub fn from_cursors(cursors: Vec<Cursor>) -> Self {
        let mut set = CursorSet { cursors };
        set.normalize();
        set
    }

    /// Add a cursor, keeping the set sorted and free of duplicate carets.
    pub fn add(&mut self, cursor: Cursor) {
        self.cursors.push(cursor);
        self.normalize();
    }

    /// The cursors in caret order.
    pub fn cursors(&self) -> &[Cursor] {
        &self.cursors
    }

    /// Number of cursors.
    pub fn len(&self) -> usize {
        self.cursors.len()
    }

    /// Whether there are no cursors.
    pub fn is_empty(&self) -> bool {
        self.cursors.is_empty()
    }

    /// Collapse to a single cursor at `pos`.
    pub fn collapse_to(&mut self, pos: usize) {
        self.cursors = vec![Cursor::at(pos)];
    }

    /// Clamp every caret and anchor into `0..=max` after an external edit.
    pub fn clamp(&mut self, max: usize) {
        for c in &mut self.cursors {
            c.caret = c.caret.min(max);
            if let Some(a) = c.anchor {
                c.anchor = Some(a.min(max));
            }
        }
        self.normalize();
    }

    fn normalize(&mut self) {
        self.cursors.sort_by_key(|c| c.caret);
        self.cursors.dedup_by_key(|c| c.caret);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_range_is_ordered() {
        let c = Cursor::selection(8, 3);
        assert_eq!(c.selected_range(), Some((3, 8)));
        assert!(c.has_selection());
    }

    #[test]
    fn empty_selection_reports_none() {
        let mut c = Cursor::at(5);
        assert_eq!(c.selected_range(), None);
        c.extend_to(5);
        assert_eq!(c.selected_range(), None);
        c.extend_to(9);
        assert_eq!(c.selected_range(), Some((5, 9)));
    }

    #[test]
    fn set_sorts_and_dedups() {
        let mut set = CursorSet::from_cursors(vec![Cursor::at(9), Cursor::at(3), Cursor::at(9)]);
        assert_eq!(set.len(), 2);
        assert_eq!(set.cursors()[0].caret, 3);
        set.add(Cursor::at(1));
        assert_eq!(set.cursors()[0].caret, 1);
    }

    #[test]
    fn clamp_pulls_into_range() {
        let mut set = CursorSet::from_cursors(vec![Cursor::at(20), Cursor::selection(2, 30)]);
        set.clamp(10);
        for c in set.cursors() {
            assert!(c.caret <= 10);
        }
    }
}
