//! Gate 3: buffer invariants and boundary safety.
//!
//! Total length and line count stay correct across a random op stream, and no
//! edit panics at a boundary: start, end, an empty buffer, or a multibyte UTF-8
//! character boundary. Character offsets can never split a multibyte sequence,
//! so every offset in `0..=char_len` is a valid edit point.

mod common;

use common::*;
use quill::PieceTable;

#[test]
fn length_and_line_count_stay_correct() {
    let ops = fuzz_ops(3000);
    let mut rng = Rng::new(0x1122_3344_5566_7788);
    let mut pt = PieceTable::empty();
    let mut reference = String::new();

    for _ in 0..ops {
        let len = ref_char_len(&reference);
        match rng.below(3) {
            0 => {
                let pos = rng.upto(len);
                let text = random_text(&mut rng);
                pt.insert(pos, &text);
                ref_insert(&mut reference, pos, &text);
            }
            1 => {
                if len > 0 {
                    let a = rng.upto(len);
                    let b = rng.upto(len);
                    pt.delete(a.min(b), a.max(b));
                    ref_delete(&mut reference, a.min(b), a.max(b));
                }
            }
            _ => {
                let a = rng.upto(len);
                let b = rng.upto(len);
                let text = random_text(&mut rng);
                pt.replace(a.min(b), a.max(b), &text);
                ref_delete(&mut reference, a.min(b), a.max(b));
                ref_insert(&mut reference, a.min(b), &text);
            }
        }
        assert_eq!(pt.char_len(), ref_char_len(&reference));
        assert_eq!(pt.byte_len(), reference.len());
        assert_eq!(pt.line_count(), ref_line_count(&reference));
    }
}

#[test]
fn edits_at_boundaries_do_not_panic() {
    // Empty buffer edits.
    let mut pt = PieceTable::empty();
    pt.insert(0, "");
    pt.delete(0, 0);
    assert!(pt.is_empty());
    assert_eq!(pt.line_count(), 1);

    // Start and end insertions.
    pt.insert(0, "middle");
    pt.insert(0, "start ");
    let end = pt.char_len();
    pt.insert(end, " end");
    assert_eq!(pt.contents(), "start middle end");

    // Delete at the very start and very end.
    pt.delete(0, 6);
    let n = pt.char_len();
    pt.delete(n - 4, n);
    assert_eq!(pt.contents(), "middle");
}

#[test]
fn multibyte_boundary_edits_are_safe() {
    let mut pt = PieceTable::new("héllo 世界 🦀");
    let n = pt.char_len();
    // Insert and delete at every character offset without panicking.
    for pos in 0..=n {
        let mut clone = pt.clone();
        clone.insert(pos, "X");
        assert_eq!(clone.char_len(), n + 1);
    }
    // Delete each single character from the front.
    while !pt.is_empty() {
        let before = pt.char_len();
        pt.delete(0, 1);
        assert_eq!(pt.char_len(), before - 1);
    }
    assert!(pt.is_empty());
}

#[test]
fn replace_across_multibyte_is_consistent() {
    let mut pt = PieceTable::new("aéb世c🦀d");
    pt.replace(1, 5, "__");
    // Reference the same operation on a String.
    let mut r = String::from("aéb世c🦀d");
    ref_delete(&mut r, 1, 5);
    ref_insert(&mut r, 1, "__");
    assert_eq!(pt.contents(), r);
}
