//! Max-scale stress harness (env scaled, small in-CI defaults).
//!
//! Every test here scales with `QUILL_FUZZ_OPS` and friends so that the default
//! `cargo test` run stays fast while a release run with large env values can
//! push millions of operations through the engine. Nothing is `#[ignore]`d.
//!
//! Knobs:
//! - `QUILL_FUZZ_OPS`: operation budget per phase (default small).
//! - `QUILL_STRESS_SEEDS`: number of seeds for the mixed differential (default 4).
//! - `QUILL_STRESS_DOC_KB`: target document size in KiB (default small).
//!
//! The oracle for the edit streams is a `Vec<char>` mirror: positions are
//! character offsets directly, splices cost O(window), and scalar counts are
//! maintained incrementally. Full `String` assembly happens only at
//! checkpoints. Piece count is asserted to be bounded by the edit count, never
//! by the document size.

mod common;

use common::*;
use quill::{PieceTable, TextEditor};

/// Target document size in bytes from `QUILL_STRESS_DOC_KB` or a default.
fn doc_target(default_kb: usize) -> usize {
    std::env::var("QUILL_STRESS_DOC_KB")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .map(|kb| kb.saturating_mul(1024))
        .unwrap_or(default_kb.saturating_mul(1024))
}

/// Number of seeds from `QUILL_STRESS_SEEDS` or a default.
fn seeds(default: usize) -> usize {
    std::env::var("QUILL_STRESS_SEEDS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(default)
        .max(1)
}

/// Builds a document full of varied content: text lines, multibyte
/// characters, and newlines. The same seed always builds the same document.
fn build_document(seed: u64, target_bytes: usize) -> String {
    let mut rng = Rng::new(seed);
    let mut doc = String::with_capacity(target_bytes + 64);
    let words = [
        "quill", "piece", "table", "buffer", "édit", "世breaks", "🦀fast", "line", "cursor",
        "undo", "résumé", "search",
    ];
    while doc.len() < target_bytes {
        for word in words.iter().take(rng.below(6) + 1) {
            doc.push_str(word);
            doc.push(' ');
        }
        doc.push('\n');
    }
    doc
}

/// One random line of 64..2048 chars mixing ASCII, newlines, and multibyte
/// characters, used for large splices.
fn random_line(rng: &mut Rng) -> String {
    const ALPHABET: &[char] = &[
        'a', 'b', 'c', ' ', '\n', '\n', 'z', '1', 'é', '世', '🦀', ' ',
    ];
    let len = 64 + rng.below(1984);
    let mut s = String::with_capacity(len * 2);
    for _ in 0..len {
        s.push(ALPHABET[rng.below(ALPHABET.len())]);
    }
    s
}

/// The `Vec<char>` oracle: character indexed mirror of the document with
/// incrementally maintained scalar counts.
struct Oracle {
    chars: Vec<char>,
    bytes: usize,
    newlines: usize,
}

impl Oracle {
    fn new(text: &str) -> Self {
        let chars: Vec<char> = text.chars().collect();
        let bytes = text.len();
        let newlines = text.matches('\n').count();
        Oracle { chars, bytes, newlines }
    }

    fn len(&self) -> usize {
        self.chars.len()
    }

    fn insert(&mut self, pos: usize, text: &str) {
        self.bytes += text.len();
        self.newlines += text.matches('\n').count();
        self.chars.splice(pos..pos, text.chars());
    }

    fn delete(&mut self, start: usize, end: usize) {
        for &c in &self.chars[start..end] {
            self.bytes -= c.len_utf8();
            if c == '\n' {
                self.newlines -= 1;
            }
        }
        self.chars.drain(start..end);
    }

    fn as_string(&self) -> String {
        self.chars.iter().collect()
    }
}

/// Full checkpoint: contents, all scalar counts, and sampled line queries.
fn full_check(pt: &PieceTable, oracle: &Oracle, step: usize) {
    let reference = oracle.as_string();
    assert_eq!(
        pt.contents(),
        reference,
        "contents diverged at checkpoint step {step}"
    );
    assert_eq!(
        pt.char_len(),
        oracle.len(),
        "char_len diverged at checkpoint step {step}"
    );
    assert_eq!(
        pt.byte_len(),
        oracle.bytes,
        "byte_len diverged at checkpoint step {step}"
    );
    assert_eq!(
        pt.line_count(),
        oracle.newlines + 1,
        "line_count diverged at checkpoint step {step}"
    );
    let total = pt.char_len();
    for &pos in &[0usize, total / 3, total / 2, total] {
        assert_eq!(
            pt.offset_to_line_col(pos),
            ref_line_col(&reference, pos),
            "offset_to_line_col({pos}) diverged at checkpoint step {step}"
        );
    }
}

/// One mixed edit against the piece table and the oracle. The mix keeps the
/// document near `target` bytes: small inserts, large splices, bounded window
/// deletes and replaces, and rare whole document rewrites.
fn mixed_op(pt: &mut PieceTable, oracle: &mut Oracle, rng: &mut Rng, target: usize, step: usize) {
    let len = oracle.len();
    let size = oracle.bytes;
    if rng.below(512) == 0 {
        // Rare adversarial whole document rewrite.
        let fresh = build_document(0xFACE_0000_u64 + step as u64, target / 2);
        pt.replace(0, len, &fresh);
        *oracle = Oracle::new(&fresh);
        return;
    }
    match rng.below(8) {
        0..=2 => {
            let pos = rng.upto(len);
            let text = random_text(rng);
            pt.insert(pos, &text);
            oracle.insert(pos, &text);
        }
        3 => {
            if size < target {
                let pos = rng.upto(len);
                let text = random_line(rng);
                pt.insert(pos, &text);
                oracle.insert(pos, &text);
            } else {
                // Shrink back toward the target with a big delete.
                let start = rng.upto(len.saturating_sub(1));
                let end = (start + 1 + rng.below(8192)).min(len);
                pt.delete(start, end);
                oracle.delete(start, end);
            }
        }
        4..=6 => {
            if len > 0 {
                let start = rng.upto(len.saturating_sub(1));
                let end = (start + 1 + rng.below(128)).min(len);
                pt.delete(start, end);
                oracle.delete(start, end);
            }
        }
        _ => {
            if len > 0 {
                let start = rng.upto(len.saturating_sub(1));
                let end = (start + 1 + rng.below(64)).min(len);
                let text = random_text(rng);
                pt.replace(start, end, &text);
                oracle.delete(start, end);
                oracle.insert(start, &text);
            }
        }
    }
}

/// The same mix driven through the editor so undo history, cursor clamping,
/// and search all run on a realistically edited buffer.
fn mixed_op_editor(
    ed: &mut TextEditor,
    oracle: &mut Oracle,
    rng: &mut Rng,
    target: usize,
    step: usize,
) {
    let len = oracle.len();
    let size = oracle.bytes;
    if rng.below(512) == 0 {
        let fresh = build_document(0xFACE_0000_u64 + step as u64, target / 2);
        ed.replace(0, len, &fresh);
        *oracle = Oracle::new(&fresh);
        return;
    }
    match rng.below(8) {
        0..=2 => {
            let pos = rng.upto(len);
            let text = random_text(rng);
            ed.insert(pos, &text);
            oracle.insert(pos, &text);
        }
        3 => {
            if size < target {
                let pos = rng.upto(len);
                let text = random_line(rng);
                ed.insert(pos, &text);
                oracle.insert(pos, &text);
            } else {
                let start = rng.upto(len.saturating_sub(1));
                let end = (start + 1 + rng.below(8192)).min(len);
                ed.delete(start, end);
                oracle.delete(start, end);
            }
        }
        4..=6 => {
            if len > 0 {
                let start = rng.upto(len.saturating_sub(1));
                let end = (start + 1 + rng.below(128)).min(len);
                ed.delete(start, end);
                oracle.delete(start, end);
            }
        }
        _ => {
            if len > 0 {
                let start = rng.upto(len.saturating_sub(1));
                let end = (start + 1 + rng.below(64)).min(len);
                let text = random_text(rng);
                ed.replace(start, end, &text);
                oracle.delete(start, end);
                oracle.insert(start, &text);
            }
        }
    }
}

/// Mixed random edit stream against a `Vec<char>` oracle on a large document.
/// Scalar counts are checked every op, full contents only at checkpoints.
#[test]
fn stress_mixed_differential_large_document() {
    let ops = fuzz_ops(4000);
    let target = doc_target(64);
    let doc = build_document(0x10CE_0001, target);
    let mut pt = PieceTable::new(&doc);
    let mut oracle = Oracle::new(&doc);
    let check_every = (ops / 40).max(1);
    let mut rng = Rng::new(0x0057_5E55_0001);

    for step in 0..ops {
        mixed_op(&mut pt, &mut oracle, &mut rng, target, step);

        assert_eq!(pt.char_len(), oracle.len(), "char_len drifted at step {step}");
        assert_eq!(
            pt.line_count(),
            oracle.newlines + 1,
            "line_count drifted at step {step}"
        );
        // Piece count must be bounded by the edit count, never by doc size.
        assert!(
            pt.piece_count() <= 2 * (step + 1) + 2,
            "piece count {} unbounded vs edits at step {step}",
            pt.piece_count()
        );

        if step % check_every == 0 || step + 1 == ops {
            full_check(&pt, &oracle, step);
        }
    }
    eprintln!(
        "stress_mixed_large: ops={ops} target_bytes={target} final_bytes={} chars={} pieces={}",
        oracle.bytes,
        pt.char_len(),
        pt.piece_count()
    );
}

/// Same mixed stream, many seeds, smaller per-seed budget: seed diversity.
#[test]
fn stress_mixed_differential_multi_seed() {
    let ops = fuzz_ops(1500);
    let count = seeds(4);
    for s in 0..count {
        let mut rng = Rng::new(0x9E37_79B9_0000_0000 + s as u64);
        let doc = build_document(0x10CE_00A0 + s as u64, 4096);
        let mut pt = PieceTable::new(&doc);
        let mut oracle = Oracle::new(&doc);
        for step in 0..ops {
            mixed_op(&mut pt, &mut oracle, &mut rng, 8192, step);
            if step % 64 == 0 {
                assert_eq!(
                    pt.contents(),
                    oracle.as_string(),
                    "seed {s} contents diverged at step {step}"
                );
            }
        }
        assert_eq!(
            pt.contents(),
            oracle.as_string(),
            "seed {s} contents diverged at end"
        );
    }
    eprintln!("stress_multi_seed: seeds={count} ops_per_seed={ops}");
}

/// Undo/redo churn: bursts of edits, exact undo count back to the pre-burst
/// state, redo back to post-burst, bounded undo/redo interleaves mid-burst,
/// redo invalidation after undo followed by a new edit.
#[test]
fn stress_undo_redo_churn() {
    let ops = fuzz_ops(3000);
    let target = doc_target(32);
    let doc = build_document(0x0DD0_0001, target);
    let mut ed = TextEditor::new(&doc);
    let mut oracle = Oracle::new(&doc);
    let mut rng = Rng::new(0xABD0_0001_C0DE);
    let burst = 200usize;
    let mut done = 0usize;
    let mut bursts = 0usize;

    while done < ops {
        bursts += 1;
        let pre_burst = oracle.as_string();
        ed.commit();
        let h0 = ed.history_len();
        for _ in 0..burst.min(ops - done) {
            let len = oracle.len();
            match rng.below(6) {
                0 => {
                    let pos = rng.upto(len);
                    let text = random_text(&mut rng);
                    ed.insert(pos, &text);
                    oracle.insert(pos, &text);
                }
                1 => {
                    if len > 0 {
                        let start = rng.upto(len.saturating_sub(1));
                        let end = (start + 1 + rng.below(96)).min(len);
                        ed.delete(start, end);
                        oracle.delete(start, end);
                    }
                }
                2 => {
                    let pos = rng.upto(len);
                    let text = random_text(&mut rng);
                    ed.type_text(pos, &text);
                    oracle.insert(pos, &text);
                }
                3 => {
                    // Bounded undo/redo interleave: content neutral.
                    let steps = rng.below(6);
                    let mut taken = 0usize;
                    for _ in 0..steps {
                        if !ed.undo() {
                            break;
                        }
                        taken += 1;
                    }
                    for _ in 0..taken {
                        assert!(ed.redo(), "redo ran dry after {taken} undos");
                    }
                }
                4 => {
                    let pos = rng.upto(len);
                    let text = random_line(&mut rng);
                    let text: String = text.chars().take(96).collect();
                    ed.insert(pos, &text);
                    oracle.insert(pos, &text);
                }
                _ => {
                    if len > 0 {
                        let start = rng.upto(len.saturating_sub(1));
                        let end = (start + 1 + rng.below(64)).min(len);
                        let text = random_text(&mut rng);
                        ed.replace(start, end, &text);
                        oracle.delete(start, end);
                        oracle.insert(start, &text);
                    }
                }
            }
            done += 1;
            assert_eq!(
                ed.char_len(),
                oracle.len(),
                "char_len drifted at churn step {done}"
            );
        }
        // Burst boundary: undo exactly the transactions this burst created and
        // the buffer must return to the pre-burst state, then redo the same
        // count returns post-burst. Comparisons run against the oracle.
        ed.commit();
        let undo_count = ed.history_len() - h0;
        assert!(
            ed.contents() == oracle.as_string(),
            "burst {bursts}: post state diverged from oracle"
        );
        for _ in 0..undo_count {
            assert!(ed.undo(), "burst {bursts}: undo ran dry");
        }
        assert!(
            ed.contents() == pre_burst,
            "burst {bursts}: undo of {undo_count} txs did not reach pre-burst"
        );
        for _ in 0..undo_count {
            assert!(ed.redo(), "burst {bursts}: redo ran dry");
        }
        assert!(
            ed.contents() == oracle.as_string(),
            "burst {bursts}: redo did not restore post-burst"
        );
        // Undo once, verify redo is available, restore, then a fresh edit that
        // must leave no redo behind.
        ed.undo();
        assert!(ed.can_redo(), "burst {bursts}: redo lost after a single undo");
        ed.redo();
        assert!(
            ed.contents() == oracle.as_string(),
            "burst {bursts}: redo after undo did not restore"
        );
        let len = oracle.len();
        let pos = rng.upto(len);
        let text = random_text(&mut rng);
        ed.insert(pos, &text);
        oracle.insert(pos, &text);
        assert!(!ed.can_redo(), "burst {bursts}: redo survived a new edit");
    }
    eprintln!(
        "stress_churn: ops={ops} bursts={bursts} final_bytes={} pieces={}",
        oracle.bytes,
        ed.buffer().piece_count()
    );
}

/// Builds a map from byte offset (`0..=len`) to the character index that
/// contains it. Boundary bytes map to the character starting there, bytes
/// inside a multibyte sequence map to the character they belong to, and the
/// final offset maps past the last character.
fn byte_to_char_map(doc: &str) -> Vec<usize> {
    let total = doc.chars().count();
    let mut map = Vec::with_capacity(doc.len() + 1);
    for (ci, (bi, _)) in doc.char_indices().enumerate() {
        while map.len() < bi {
            map.push(ci - 1);
        }
        map.push(ci);
    }
    while map.len() < doc.len() {
        map.push(total - 1);
    }
    map.push(total);
    map
}

/// Boundary and multibyte assault: edits at every character offset of a
/// multibyte document, replace with empty, delete the whole document then edit
/// again, and replace ranges spanning the entire document.
#[test]
fn stress_boundary_and_multibyte_assault() {
    let ops = fuzz_ops(600);
    let unit = "aé世🦀\nbé世🦀\n";
    // A small dense multibyte document: every offset is exercised by cloning.
    let mut doc = String::new();
    while doc.len() < 512 {
        doc.push_str(unit);
    }
    let mut rng = Rng::new(0xB0CA_0001);

    for _ in 0..ops {
        let pt = PieceTable::new(&doc);
        let n = pt.char_len();
        // Insert at every offset on a clone; contents must match the oracle.
        for pos in 0..=n {
            let mut clone = pt.clone();
            clone.insert(pos, "Xé");
            let mut expect = doc.clone();
            ref_insert(&mut expect, pos, "Xé");
            assert_eq!(clone.contents(), expect, "insert at {pos} diverged");
        }
        // Delete one character at every offset on a clone.
        for pos in 0..n {
            let mut clone = pt.clone();
            clone.delete(pos, pos + 1);
            let mut expect = doc.clone();
            ref_delete(&mut expect, pos, pos + 1);
            assert_eq!(clone.contents(), expect, "delete at {pos} diverged");
        }
        // Multibyte split attempt at every BYTE offset: map the byte to the
        // character offset that contains it, then edit there. A char indexed
        // buffer can never split a sequence, so every byte offset must produce
        // the reference result of editing the containing character.
        let map = byte_to_char_map(&doc);
        for (b, &char_pos) in map.iter().enumerate() {
            let mut clone = pt.clone();
            clone.replace(char_pos, (char_pos + 1).min(n), "世");
            let mut expect = doc.clone();
            ref_delete(&mut expect, char_pos, (char_pos + 1).min(n));
            ref_insert(&mut expect, char_pos, "世");
            assert_eq!(clone.contents(), expect, "byte offset {b} edit diverged");
        }
        // Replace the whole document, then replace with empty (pure delete),
        // then delete everything and edit again.
        let mut live = pt;
        let n = live.char_len();
        live.replace(0, n, &build_document(0xB0CA_00F0 + rng.next_u64() % 8, 256));
        let n2 = live.char_len();
        live.replace(0, n2, "");
        assert!(live.is_empty());
        live.insert(0, "again é世🦀");
        let n3 = live.char_len();
        live.delete(0, n3);
        assert!(live.is_empty());
        live.insert(0, "fresh");
        doc = live.contents();
    }
    assert!(!doc.is_empty());
    eprintln!("stress_boundary: iterations={ops} final_bytes={}", doc.len());
}

/// Search edge cases at scale: empty needle, needle longer than the document,
/// overlapping matches, multibyte needles, and matches spanning piece
/// boundaries after heavy edits.
#[test]
fn stress_search_edges() {
    let ops = fuzz_ops(2000);
    let target = doc_target(64);
    let doc = build_document(0x5EA7_0001, target);
    let mut ed = TextEditor::new(&doc);
    let mut rng = Rng::new(0x5EA7_7001);

    // Naive non-overlapping reference finder over characters.
    fn naive_find_all(hay: &[char], needle: &[char]) -> Vec<usize> {
        let mut out = Vec::new();
        if needle.is_empty() {
            return out;
        }
        let mut i = 0usize;
        while i + needle.len() <= hay.len() {
            if hay[i..i + needle.len()] == *needle {
                out.push(i);
                i += needle.len();
            } else {
                i += 1;
            }
        }
        out
    }

    assert!(quill::find_all(&doc, "").is_empty(), "empty needle matched");
    assert!(
        quill::find_all(&doc, &format!("{doc}!")).is_empty(),
        "needle longer than doc matched"
    );
    assert_eq!(quill::find_all("aaaa", "aa"), vec![0, 2]);
    assert_eq!(quill::find_all("aaaaa", "aaa"), vec![0]);
    assert_eq!(quill::find_all("aaaaaa", "aaa"), vec![0, 3]);
    assert_eq!(quill::find_all("é世🦀é世🦀", "世🦀"), vec![1, 4]);

    let needles = ["quill", "é", "世breaks", "🦀fast", "e line", "\n", "  ", "世breaks 🦀"];
    let needle_chars: Vec<Vec<char>> = needles.iter().map(|n| n.chars().collect()).collect();
    let mut oracle = Oracle::new(&doc);
    for step in 0..ops {
        mixed_op_editor(&mut ed, &mut oracle, &mut rng, target, step);
        if step % 32 == 0 {
            let contents = oracle.as_string();
            let hay: Vec<char> = contents.chars().collect();
            for (i, needle) in needle_chars.iter().enumerate() {
                assert_eq!(
                    ed.find_all(needles[i]),
                    naive_find_all(&hay, needle),
                    "search {:?} diverged at step {step}",
                    needles[i]
                );
            }
        }
    }
    let contents = oracle.as_string();
    let hay: Vec<char> = contents.chars().collect();
    for (i, needle) in needle_chars.iter().enumerate() {
        assert_eq!(
            ed.find_all(needles[i]),
            naive_find_all(&hay, needle),
            "search {:?} diverged at end",
            needles[i]
        );
    }
    assert!(ed.find_all("").is_empty());
    eprintln!("stress_search: ops={ops} final_chars={}", oracle.len());
}

/// Piece count must not grow without bound under delete/insert churn at
/// scattered fresh positions: the seam left by a delete is merged back so
/// steady state editing does not inflate the piece list.
#[test]
fn piece_count_bounded_under_churn() {
    let cycles = fuzz_ops(2000);
    let mut rng = Rng::new(0x9EAB_C001);
    let doc = build_document(0x9EAB_0001, 4096);
    let mut pt = PieceTable::new(&doc);
    for _ in 0..cycles {
        let len = pt.char_len();
        let pos = rng.upto(len.saturating_sub(4));
        pt.insert(pos, "xyz");
        pt.delete(pos, pos + 3);
    }
    assert!(
        pt.piece_count() <= 8,
        "piece count {} exploded after {cycles} scattered type/delete cycles",
        pt.piece_count()
    );
    assert_eq!(pt.contents(), doc, "churn changed the contents");
    eprintln!(
        "stress_churn_pieces: cycles={cycles} pieces={}",
        pt.piece_count()
    );
}
