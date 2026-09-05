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
use quill::TextEditor;

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
