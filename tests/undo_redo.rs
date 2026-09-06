//! Gate 2: undo/redo round trips.
//!
//! Three properties are checked. First, from an initial buffer a random op
//! sequence followed by undoing everything returns the buffer to its exact
//! initial state, and redoing everything returns it to the post edit state.
//! Second, a mixed random walk of undo and redo stays consistent with a
//! reference list of every intermediate state. Third, a new edit made after an
//! undo invalidates redo.

mod common;

use common::*;
use quill::{Cursor, CursorSet, TextEditor};

#[test]
fn undo_all_then_redo_all_round_trip() {
    let ops = fuzz_ops(1500);
    let mut rng = Rng::new(0x5EED_1234_ABCD_9999);
    let initial = "alpha\nbeta\ngamma\n";
    let mut ed = TextEditor::new(initial);

    for _ in 0..ops {
        let len = ed.char_len();
        match rng.below(3) {
            0 => {
                let pos = rng.upto(len);
                ed.insert(pos, &random_text(&mut rng));
            }
            1 => {
                if len > 0 {
                    let a = rng.upto(len);
                    let b = rng.upto(len);
                    ed.delete(a.min(b), a.max(b));
                }
            }
            _ => {
                let a = rng.upto(len);
                let b = rng.upto(len);
                ed.replace(a.min(b), a.max(b), &random_text(&mut rng));
            }
        }
    }

    let post = ed.contents();
    while ed.undo() {}
    assert_eq!(ed.contents(), initial, "undo did not restore initial state");
    while ed.redo() {}
    assert_eq!(ed.contents(), post, "redo did not restore post edit state");
}

#[test]
fn mixed_undo_redo_matches_reference_states() {
    let steps = fuzz_ops(400);
    let mut rng = Rng::new(0xF00D_C0DE_1357_2468);
    let mut ed = TextEditor::new("start");

    // Record the state after every op that actually created an undo group. An
    // op that changes nothing (an empty insert, an empty range) records no
    // group, so it must not add a reference state either.
    let mut states = vec![ed.contents()];
    for _ in 0..steps {
        let len = ed.char_len();
        let before = ed.history_len();
        match rng.below(3) {
            0 => ed.insert(rng.upto(len), &random_text(&mut rng)),
            1 => {
                if len > 0 {
                    let a = rng.upto(len);
                    let b = rng.upto(len);
                    ed.delete(a.min(b), a.max(b));
                } else {
                    ed.insert(0, "x");
                }
            }
            _ => {
                let a = rng.upto(len);
                let b = rng.upto(len);
                ed.replace(a.min(b), a.max(b), &random_text(&mut rng));
            }
        }
        if ed.history_len() > before {
            states.push(ed.contents());
        }
    }

    // Random walk of undo/redo, checking the buffer against the recorded state.
    let mut idx = states.len() - 1;
    for _ in 0..(steps * 2) {
        if rng.below(2) == 0 {
            if ed.undo() {
                idx -= 1;
            }
        } else if ed.redo() {
            idx += 1;
        }
        assert_eq!(
            ed.contents(),
            states[idx],
            "mixed undo/redo diverged at history index {idx}"
        );
    }
}

#[test]
fn new_edit_after_undo_invalidates_redo() {
    let mut ed = TextEditor::new("abcdef");
    ed.delete(0, 2);
    ed.delete(0, 2);
    assert!(ed.undo());
    assert!(ed.undo());
    assert_eq!(ed.contents(), "abcdef");
    ed.insert(0, "Z");
    assert!(!ed.can_redo());
    assert_eq!(ed.contents(), "Zabcdef");
}

#[test]
fn undo_past_the_beginning_stops_cleanly() {
    let mut ed = TextEditor::new("seed");
    ed.insert(4, " more");
    assert!(ed.undo());
    // The history is exhausted: further undo calls report false and leave the
    // buffer untouched instead of panicking or corrupting state.
    for _ in 0..8 {
        assert!(!ed.undo());
        assert_eq!(ed.contents(), "seed");
    }
    // Redo still works after a run of dry undos.
    assert!(ed.redo());
    assert_eq!(ed.contents(), "seed more");
}

#[test]
fn redo_after_history_truncation_is_dead() {
    let mut ed = TextEditor::new("seed");
    ed.insert(4, " one");
    ed.insert(8, " two");
    // Build redo candidates, then truncate them with a fresh edit.
    assert!(ed.undo());
    assert!(ed.can_redo());
    ed.insert(0, "NEW ");
    assert!(!ed.can_redo());
    for _ in 0..4 {
        assert!(!ed.redo());
        assert_eq!(ed.contents(), "NEW seed one");
    }
    // Truncation mid-walk: undo twice, redo once, new edit kills the rest.
    let mut ed = TextEditor::new("base");
    ed.insert(4, "1");
    ed.insert(5, "2");
    ed.insert(6, "3");
    assert!(ed.undo());
    assert!(ed.undo());
    assert!(ed.redo());
    assert_eq!(ed.contents(), "base12");
    ed.insert(6, "X");
    assert!(!ed.can_redo());
    while ed.undo() {}
    assert_eq!(ed.contents(), "base");
    while ed.redo() {}
    assert_eq!(ed.contents(), "base12X");
}

#[test]
fn empty_document_edit_cycle_round_trips() {
    let mut ed = TextEditor::empty();
    // Undo/redo on an empty history must be clean.
    assert!(!ed.undo());
    assert!(!ed.redo());
    // Delete the entire document, then edit again.
    ed.insert(0, "content é世🦀");
    let n = ed.char_len();
    ed.delete(0, n);
    assert!(ed.is_empty());
    assert_eq!(ed.line_count(), 1);
    ed.insert(0, "again");
    ed.insert(5, "!");
    assert_eq!(ed.contents(), "again!");
    // Replace the whole document with the empty string (pure delete).
    let n = ed.char_len();
    ed.replace(0, n, "");
    assert!(ed.is_empty());
    assert!(ed.undo(), "replace recorded no undo group");
    assert_eq!(ed.contents(), "again!");
    while ed.undo() {}
    assert!(ed.is_empty(), "undo-all must reach the empty initial document");
    // A no-op replace records no group and no redo invalidation.
    let mut ed = TextEditor::new("keep");
    ed.replace(2, 2, "");
    assert_eq!(ed.history_len(), 0);
    assert_eq!(ed.contents(), "keep");
}

#[test]
fn cursor_ranges_at_document_boundaries() {
    let mut ed = TextEditor::new("seed é世🦀");
    let len = ed.char_len();
    // Cursors at both boundaries and beyond: clamped into range.
    ed.set_cursors(CursorSet::from_cursors(vec![
        Cursor::at(0),
        Cursor::at(len),
        Cursor::at(len + 100),
        Cursor::selection(0, len),
    ]));
    for c in ed.cursors().cursors() {
        assert!(c.caret <= len, "caret {} past end {len}", c.caret);
    }
    // Multi-cursor insert with a boundary caret set.
    let mut ed = TextEditor::new("mid");
    ed.set_cursors(CursorSet::from_cursors(vec![Cursor::at(0), Cursor::at(3)]));
    assert_eq!(ed.insert_at_cursors("<>"), 2);
    assert_eq!(ed.contents(), "<>mid<>");
    // Undo walks one cursor at a time: the lowest caret was applied last, so
    // its transaction is reverted first.
    assert!(ed.undo());
    assert_eq!(ed.contents(), "mid<>");
    assert!(ed.undo());
    assert_eq!(ed.contents(), "mid");
    assert!(!ed.undo());
    // Delete a selection spanning the whole document from the cursor model.
    let mut ed = TextEditor::new("whole");
    ed.set_cursors(CursorSet::from_cursors(vec![Cursor::selection(0, 5)]));
    assert_eq!(ed.delete_selections(), 1);
    assert!(ed.is_empty());
    assert_eq!(ed.cursors().cursors()[0].caret, 0);
    assert!(ed.undo());
    assert_eq!(ed.contents(), "whole");
    // replace_all on an empty document and with a needle equal to the document.
    let mut ed = TextEditor::empty();
    assert_eq!(ed.replace_all("x", "y"), 0);
    assert_eq!(ed.replace_next("x", "y", 0), None);
    ed.insert(0, "solo");
    assert_eq!(ed.replace_all("solo", ""), 1);
    assert!(ed.is_empty());
    assert!(ed.undo());
    assert_eq!(ed.contents(), "solo");
}
