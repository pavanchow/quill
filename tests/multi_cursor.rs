//! Gate 4: multi-cursor editing and structured search-replace.
//!
//! Multi-cursor results are compared against independently computed expected
//! text: the same per-cursor edits composed as single-cursor byte splices on a
//! plain `String`, applied in the same descending order. Undo grouping is
//! checked per cursor, and replace-all is checked against a naive character
//! scan. A performance invariant pins piece count to the edit count, never the
//! document size.

mod common;

use common::*;
use quill::{Cursor, CursorSet, TextEditor};

/// Byte index of the `k`th character boundary in `s` (already in common, but
/// re-imported for the splice oracle below).
fn splice(s: &mut String, at_char: usize, text: &str) {
    ref_insert(s, at_char, text);
}

/// Apply the same insert the engine applies at each caret, descending, onto a
/// plain String. `points` must be sorted ascending.
fn oracle_insert_all(initial: &str, points: &[usize], text: &str) -> String {
    let mut out = initial.to_string();
    for &at in points.iter().rev() {
        splice(&mut out, at, text);
    }
    out
}

/// Oracle state after undoing `undone` lowest cursors: edits at
/// `points[undone..]` are still applied.
fn oracle_state(initial: &str, points: &[usize], text: &str, undone: usize) -> String {
    let mut out = initial.to_string();
    for &at in points[undone..].iter().rev() {
        splice(&mut out, at, text);
    }
    out
}

/// Replace every selection `(s, e)` with `text`, descending, on a String.
fn oracle_replace_selections(initial: &str, sels: &[(usize, usize)], text: &str) -> String {
    let mut out = initial.to_string();
    for &(s, e) in sels.iter().rev() {
        let bs = char_to_byte(&out, s);
        let be = char_to_byte(&out, e);
        out.replace_range(bs..be, text);
    }
    out
}

/// Delete every selection `(s, e)`, descending, on a String.
fn oracle_delete_selections(initial: &str, sels: &[(usize, usize)]) -> String {
    let mut out = initial.to_string();
    for &(s, e) in sels.iter().rev() {
        ref_delete(&mut out, s, e);
    }
    out
}

/// Naive non-overlapping left-to-right replace over characters.
fn naive_replace_all(hay: &str, needle: &str, repl: &str) -> (String, usize) {
    let ch: Vec<char> = hay.chars().collect();
    let pat: Vec<char> = needle.chars().collect();
    let rep: Vec<char> = repl.chars().collect();
    let mut out: Vec<char> = Vec::with_capacity(ch.len());
    let mut i = 0usize;
    let mut count = 0usize;
    while i < ch.len() {
        if !pat.is_empty() && i + pat.len() <= ch.len() && ch[i..i + pat.len()] == pat[..] {
            out.extend_from_slice(&rep);
            i += pat.len();
            count += 1;
        } else {
            out.push(ch[i]);
            i += 1;
        }
    }
    (out.into_iter().collect(), count)
}

#[test]
fn multi_cursor_insert_matches_single_cursor_oracle() {
    let steps = fuzz_ops(60);
    let mut rng = Rng::new(0x9CCE_0001);
    let initial = build_seed();

    for step in 0..steps {
        let mut ed = TextEditor::new(&initial);
        let len = ref_char_len(&initial);
        // Ascending distinct carets.
        let k = 1 + rng.below(5);
        let mut points = std::collections::BTreeSet::new();
        while points.len() < k {
            points.insert(rng.upto(len));
        }
        let points: Vec<usize> = points.into_iter().collect();
        ed.set_cursors(CursorSet::from_cursors(
            points.iter().map(|&p| Cursor::at(p)).collect(),
        ));

        let mut text = random_text(&mut rng);
        while text.is_empty() {
            text = random_text(&mut rng);
        }
        let expected = oracle_insert_all(&initial, &points, &text);
        let applied = ed.insert_at_cursors(&text);
        assert_eq!(applied, points.len(), "step {step}: wrong applied count");
        assert_eq!(
            ed.contents(),
            expected,
            "step {step}: multi-cursor insert diverged from oracle"
        );

        // Carets sit just past their own copy, shifted by lower inserts.
        let n = text.chars().count();
        for (i, c) in ed.cursors().cursors().iter().enumerate() {
            let want = points[i] + n * (i + 1);
            assert_eq!(c.caret, want, "step {step}: caret {i} misplaced");
        }

        // Per-cursor undo grouping: k transactions, states match the oracle.
        assert_eq!(
            ed.history_len(),
            points.len(),
            "step {step}: expected one transaction per cursor"
        );
        let mut undone = 0usize;
        while ed.undo() {
            undone += 1;
            let want = oracle_state(&initial, &points, &text, undone);
            assert_eq!(
                ed.contents(),
                want,
                "step {step}: undo state {undone} diverged"
            );
        }
        assert_eq!(undone, points.len(), "step {step}: undo count mismatch");
        assert_eq!(ed.contents(), initial, "step {step}: full undo mismatch");

        // Redo rebuilds the same result.
        while ed.redo() {}
        assert_eq!(ed.contents(), expected, "step {step}: redo mismatch");
    }
}

#[test]
fn multi_cursor_selection_replace_matches_oracle() {
    let steps = fuzz_ops(60);
    let mut rng = Rng::new(0x9CCE_0002);
    let initial = build_seed();

    for step in 0..steps {
        let mut ed = TextEditor::new(&initial);
        let len = ref_char_len(&initial);
        // Ascending non-overlapping selections.
        let mut sels: Vec<(usize, usize)> = Vec::new();
        let mut cursor = 0usize;
        let count = 1 + rng.below(4);
        for _ in 0..count {
            if cursor + 2 > len {
                break;
            }
            let s = cursor + rng.upto(len - cursor - 1);
            let width = 1 + rng.below((len - s).min(8));
            let e = s + width;
            sels.push((s, e));
            cursor = e + 2;
        }
        if sels.is_empty() {
            continue;
        }
        ed.set_cursors(CursorSet::from_cursors(
            sels.iter().map(|&(s, e)| Cursor::selection(s, e)).collect(),
        ));

        let mut text = random_text(&mut rng);
        while text.is_empty() {
            text = random_text(&mut rng);
        }
        let expected = oracle_replace_selections(&initial, &sels, &text);
        let applied = ed.insert_at_cursors(&text);
        assert_eq!(applied, sels.len(), "step {step}: wrong applied count");
        assert_eq!(
            ed.contents(),
            expected,
            "step {step}: selection replace diverged"
        );

        // One undo per cursor restores the initial text step by step.
        let mut undos = 0usize;
        while ed.undo() {
            undos += 1;
        }
        assert_eq!(undos, sels.len(), "step {step}: undo grouping broken");
        assert_eq!(ed.contents(), initial, "step {step}: undo did not restore");
        while ed.redo() {}
        assert_eq!(ed.contents(), expected, "step {step}: redo did not rebuild");
    }
}

#[test]
fn delete_selections_matches_oracle() {
    let steps = fuzz_ops(60);
    let mut rng = Rng::new(0x9CCE_0003);
    let initial = build_seed();

    for step in 0..steps {
        let mut ed = TextEditor::new(&initial);
        let len = ref_char_len(&initial);
        let mut sels: Vec<(usize, usize)> = Vec::new();
        let mut bare: Vec<usize> = Vec::new();
        let mut cursor = 0usize;
        for _ in 0..4 {
            if cursor + 2 > len {
                break;
            }
            if rng.below(2) == 0 {
                let s = cursor + rng.upto(len - cursor - 1);
                let e = s + 1 + rng.below((len - s).min(8));
                sels.push((s, e));
                cursor = e + 2;
            } else {
                let p = cursor + rng.upto(len - cursor);
                bare.push(p);
                cursor = p + 2;
            }
        }
        if sels.is_empty() {
            continue;
        }
        let mut cursors: Vec<Cursor> = sels.iter().map(|&(s, e)| Cursor::selection(s, e)).collect();
        cursors.extend(bare.iter().map(|&p| Cursor::at(p)));
        ed.set_cursors(CursorSet::from_cursors(cursors));

        let expected = oracle_delete_selections(&initial, &sels);
        let applied = ed.delete_selections();
        assert_eq!(applied, sels.len(), "step {step}: wrong deletion count");
        assert_eq!(
            ed.contents(),
            expected,
            "step {step}: delete_selections diverged"
        );

        // One undo per deletion restores everything.
        let mut undos = 0usize;
        while ed.undo() {
            undos += 1;
        }
        assert_eq!(undos, sels.len(), "step {step}: undo grouping broken");
        assert_eq!(ed.contents(), initial, "step {step}: undo did not restore");
    }
}

#[test]
fn replace_all_matches_naive_scan() {
    let steps = fuzz_ops(80);
    let mut rng = Rng::new(0x9CCE_0004);
    let initial = build_seed();

    assert_eq!(TextEditor::new("x").replace_all("", "y"), 0, "empty needle");
    let mut ed = TextEditor::new("abc abc");
    assert_eq!(ed.replace_all("abc", "abc"), 2, "identity replace");
    assert_eq!(ed.contents(), "abc abc");
    let mut ed = TextEditor::new("aaaa");
    assert_eq!(ed.replace_all("aa", "b"), 2, "overlap consume");
    assert_eq!(ed.contents(), "bb");
    let mut ed = TextEditor::new("hello");
    assert_eq!(ed.replace_all("zz", "y"), 0, "missing needle");
    assert_eq!(ed.contents(), "hello");
    let mut ed = TextEditor::new("hello");
    assert_eq!(ed.replace_all("hello", "hi"), 1, "whole doc needle");
    assert_eq!(ed.contents(), "hi");
    let mut ed = TextEditor::new("abcXabc");
    assert_eq!(ed.replace_all("abc", "xab"), 2, "replacement contains needle");
    assert_eq!(ed.contents(), "xabXxab");

    for step in 0..steps {
        let mut ed = TextEditor::new(&initial);
        // Random needle from the document's own alphabet.
        let needle: String = {
            let n = 1 + rng.below(4);
            (0..n).map(|_| random_text(&mut rng)).collect()
        };
        if needle.is_empty() {
            continue;
        }
        let replacement: String = {
            let n = rng.below(5);
            (0..n).map(|_| random_text(&mut rng)).collect()
        };
        let (expected, count) = naive_replace_all(&initial, &needle, &replacement);
        let applied = ed.replace_all(&needle, &replacement);
        assert_eq!(applied, count, "step {step}: wrong replace count");
        assert_eq!(
            ed.contents(),
            expected,
            "step {step}: replace_all({needle:?}, {replacement:?}) diverged"
        );

        // Single undo group semantics (only when something was replaced).
        if count > 0 {
            assert!(
                ed.history_len() == 1,
                "step {step}: replace_all must be exactly one undo group"
            );
            assert!(ed.undo(), "step {step}: replace_all recorded no undo group");
            assert_eq!(ed.contents(), initial, "step {step}: undo did not restore");
            assert!(ed.redo(), "step {step}: no redo after undo");
            assert_eq!(ed.contents(), expected, "step {step}: redo mismatch");
        } else {
            assert_eq!(
                ed.history_len(),
                0,
                "step {step}: no-op replace_all recorded history"
            );
        }
    }
}

#[test]
fn replace_next_interactive_loop_equals_replace_all() {
    let steps = fuzz_ops(40);
    let mut rng = Rng::new(0x9CCE_0005);
    let initial = build_seed();

    // Deterministic semantics first.
    let mut ed = TextEditor::new("aXbXc");
    assert_eq!(ed.replace_next("X", "-", 0), Some((1, 2)));
    assert_eq!(ed.contents(), "a-bXc");
    assert_eq!(ed.replace_next("X", "-", 0), Some((3, 4)));
    assert_eq!(ed.contents(), "a-b-c");
    assert_eq!(ed.replace_next("X", "-", 0), None);
    assert_eq!(ed.replace_next("", "-", 0), None, "empty needle");
    let mut ed = TextEditor::new("aXbXc");
    assert_eq!(ed.replace_next("X", "-", 2), Some((3, 4)), "from offset");

    for _ in 0..steps {
        let needle: String = (0..=rng.below(3))
            .map(|_| random_text(&mut rng))
            .collect();
        if needle.is_empty() {
            continue;
        }
        let replacement: String = (0..rng.below(4))
            .map(|_| random_text(&mut rng))
            .collect();

        // replace_all as the expected outcome.
        let mut all = TextEditor::new(&initial);
        let (expected, count) = naive_replace_all(&initial, &needle, &replacement);
        assert_eq!(all.replace_all(&needle, &replacement), count);

        // Interactive loop: one replace_next at a time, advancing past the
        // inserted replacement, one undo group per replacement.
        let mut next = TextEditor::new(&initial);
        let mut from = 0usize;
        let mut count2 = 0usize;
        while let Some((_s, e)) = next.replace_next(&needle, &replacement, from) {
            from = e;
            count2 += 1;
            assert!(count2 <= count + 1, "interactive loop overran");
        }
        assert_eq!(count2, count, "interactive count mismatch");
        assert_eq!(
            next.contents(),
            expected,
            "interactive loop diverged from replace_all"
        );

        // Every interactive step is its own undo group.
        let mut undos = 0usize;
        while next.undo() {
            undos += 1;
        }
        assert_eq!(undos, count, "undo grouping per interactive step broken");
        assert_eq!(next.contents(), initial);
    }
}

/// A bare caret strictly inside another cursor's selection is consumed by that
/// selection's edit. For `delete_selections` the documented contract is that the
/// caret collapses onto the deleted range's start (never below it). For
/// `insert_at_cursors` the caret is dropped: its insert would land inside text
/// that the replacement is about to delete, shifting the delete range and
/// corrupting the result.
#[test]
fn caret_inside_a_selection_is_consumed_not_corrupted() {
    // delete_selections: the swallowed caret collapses to the range start.
    let mut ed = TextEditor::new("0123456789");
    ed.set_cursors(CursorSet::from_cursors(vec![
        Cursor::selection(2, 7),
        Cursor::at(4),
    ]));
    assert_eq!(ed.delete_selections(), 1);
    assert_eq!(ed.contents(), "01789");
    let carets: Vec<usize> = ed.cursors().cursors().iter().map(|c| c.caret).collect();
    assert_eq!(
        carets,
        vec![2],
        "caret inside a deleted range must collapse to the range start"
    );
    assert!(ed.undo());
    assert_eq!(ed.contents(), "0123456789");

    // Same shape, caret at the range end: the plain shift already lands on the
    // start, and both cursors merge there.
    let mut ed = TextEditor::new("0123456789");
    ed.set_cursors(CursorSet::from_cursors(vec![
        Cursor::selection(2, 7),
        Cursor::at(7),
    ]));
    assert_eq!(ed.delete_selections(), 1);
    assert_eq!(ed.contents(), "01789");
    let carets: Vec<usize> = ed.cursors().cursors().iter().map(|c| c.caret).collect();
    assert_eq!(carets, vec![2], "caret at the range end shifts onto the start");

    // insert_at_cursors: the caret inside the replaced range is consumed by
    // the replacement, so the result is exactly the single-selection oracle.
    let mut ed = TextEditor::new("0123456789");
    ed.set_cursors(CursorSet::from_cursors(vec![
        Cursor::selection(2, 7),
        Cursor::at(4),
    ]));
    assert_eq!(ed.insert_at_cursors("[]"), 1, "only the selection edits");
    let mut expect = String::from("0123456789");
    ref_delete(&mut expect, 2, 7);
    ref_insert(&mut expect, 2, "[]");
    assert_eq!(ed.contents(), expect, "swallowed caret corrupted the text");
    assert_eq!(ed.cursors().cursors()[0].caret, 4, "caret past its copy");
    assert_eq!(ed.history_len(), 1, "one transaction for the one edit");
    assert!(ed.undo());
    assert_eq!(ed.contents(), "0123456789");
    assert!(ed.redo());
    assert_eq!(ed.contents(), expect);
}

#[test]
fn piece_count_bounded_by_edits_not_document_size() {
    // One shared edit script, defined before any statements run.
    fn run(doc: &str, edits: usize) -> usize {
        let mut ed = TextEditor::new(doc);
        let mut rng = Rng::new(0x91CE_BEEF);
        for _ in 0..edits {
            let len = ed.char_len();
            match rng.below(3) {
                0 => {
                    let pos = rng.upto(len);
                    ed.insert(pos, "piece ");
                }
                1 => {
                    if len > 2 {
                        let a = rng.upto(len - 1);
                        let b = (a + 1 + rng.below(16)).min(len);
                        ed.delete(a, b);
                    }
                }
                _ => {
                    if len > 2 {
                        let a = rng.upto(len - 1);
                        let b = (a + 1 + rng.below(16)).min(len);
                        ed.replace(a, b, "XY");
                    }
                }
            }
        }
        ed.buffer().piece_count()
    }

    // Same character count, wildly different byte size: identical edit
    // scripts in char space must give identical piece counts. Structural
    // size tracks edits, never text volume.
    let small_doc = "abcdefghij".repeat(100 * 1024); // 1M ASCII chars, 1MB
    let huge_doc: String = small_doc.chars().map(|_| '\u{1F980}').collect(); // 4MB
    assert_eq!(ref_char_len(&small_doc), ref_char_len(&huge_doc));
    assert!(huge_doc.len() > 3 * small_doc.len());
    let edits = 400usize;

    let small_pieces = run(&small_doc, edits);
    let huge_pieces = run(&huge_doc, edits);
    assert_eq!(
        small_pieces, huge_pieces,
        "piece count depends on document size"
    );
    assert!(
        huge_pieces <= 2 * edits + 2,
        "piece count {huge_pieces} exceeds the edit bound"
    );
    eprintln!(
        "depth_piece_bound: edits={edits} pieces_small={small_pieces} pieces_huge={huge_pieces}"
    );
}

/// Shared seed document builder for this gate file.
fn build_seed() -> String {
    let mut s = String::from("alpha beta gamma delta é世🦀\n");
    for i in 0..12 {
        s.push_str("line ");
        s.push_str(&i.to_string());
        s.push_str(" of the seed document é世🦀\n");
    }
    s
}
