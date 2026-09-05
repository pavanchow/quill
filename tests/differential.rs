//! Gate 1: differential test against a naive `String` buffer.
//!
//! The same random stream of insert, delete, and replace operations is applied
//! to the piece table and to a plain `String`. After every operation the full
//! contents must match, and so must every line and column query: total length,
//! line count, offset to line/col over sampled offsets, line/col to offset, and
//! the slice of every line. Any divergence fails the gate.

mod common;

use common::*;
use quill::PieceTable;

#[test]
fn differential_against_string_reference() {
    let ops = fuzz_ops(3000);
    let mut rng = Rng::new(0xC1FE_11A5_51CE_D00D);
    let mut pt = PieceTable::new("The quick brown fox\njumped over\nthe lazy dog\n");
    let mut reference = pt.contents();

    for step in 0..ops {
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
                    let (start, end) = (a.min(b), a.max(b));
                    pt.delete(start, end);
                    ref_delete(&mut reference, start, end);
                }
            }
            _ => {
                let a = rng.upto(len);
                let b = rng.upto(len);
                let (start, end) = (a.min(b), a.max(b));
                let text = random_text(&mut rng);
                pt.replace(start, end, &text);
                ref_delete(&mut reference, start, end);
                ref_insert(&mut reference, start, &text);
            }
        }

        // Contents and coarse counts.
        assert_eq!(
            pt.contents(),
            reference,
            "contents diverged at step {step}"
        );
        assert_eq!(
            pt.char_len(),
            ref_char_len(&reference),
            "char_len diverged at step {step}"
        );
        assert_eq!(
            pt.line_count(),
            ref_line_count(&reference),
            "line_count diverged at step {step}"
        );

        // Line and column queries at sampled offsets.
        let total = pt.char_len();
        let samples = [
            0,
            total,
            total / 2,
            rng.upto(total),
            rng.upto(total),
            rng.upto(total),
        ];
        for &pos in &samples {
            assert_eq!(
                pt.offset_to_line_col(pos),
                ref_line_col(&reference, pos),
                "offset_to_line_col({pos}) diverged at step {step}"
            );
        }

        // Every line: start offset, slice, and a line/col round trip.
        for line in 0..pt.line_count() {
            assert_eq!(
                pt.line_start_offset(line),
                ref_line_start(&reference, line),
                "line_start_offset({line}) diverged at step {step}"
            );
            assert_eq!(
                pt.line_slice(line),
                ref_line_slice(&reference, line),
                "line_slice({line}) diverged at step {step}"
            );
            for &col in &[0usize, 1, 3] {
                assert_eq!(
                    pt.line_col_to_offset(line, col),
                    ref_line_col_to_offset(&reference, line, col),
                    "line_col_to_offset({line},{col}) diverged at step {step}"
                );
            }
        }
    }
}
