//! Shared test helpers: a tiny dependency free PRNG and a naive `String` based
//! reference implementation of every buffer query the gate checks.

// This module is included by several test binaries; not every binary uses every
// helper, so per-binary dead code warnings here are expected and silenced.
#![allow(dead_code)]

/// SplitMix64: a small, well distributed PRNG so the fuzz gates need no crates.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A number in `0..n` (`0` when `n == 0`).
    pub fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next_u64() % n as u64) as usize
        }
    }

    /// An inclusive `0..=n`.
    pub fn upto(&mut self, n: usize) -> usize {
        self.below(n + 1)
    }
}

/// Number of operations for a gate, from `QUILL_FUZZ_OPS` or `default`.
pub fn fuzz_ops(default: usize) -> usize {
    std::env::var("QUILL_FUZZ_OPS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(default)
}

/// A short random string drawn from an alphabet that includes newlines and
/// multibyte characters, so UTF-8 handling is exercised.
pub fn random_text(rng: &mut Rng) -> String {
    const ALPHABET: &[char] = &[
        'a', 'b', 'c', ' ', '\n', 'z', '1', 'é', '世', '🦀',
    ];
    let len = rng.below(6);
    let mut s = String::new();
    for _ in 0..len {
        s.push(ALPHABET[rng.below(ALPHABET.len())]);
    }
    s
}

/// Byte index of the `k`th character boundary in `s`.
pub fn char_to_byte(s: &str, k: usize) -> usize {
    s.char_indices().nth(k).map(|(b, _)| b).unwrap_or(s.len())
}

pub fn ref_char_len(s: &str) -> usize {
    s.chars().count()
}

pub fn ref_insert(s: &mut String, pos_char: usize, text: &str) {
    let b = char_to_byte(s, pos_char);
    s.insert_str(b, text);
}

pub fn ref_delete(s: &mut String, start_char: usize, end_char: usize) {
    let bs = char_to_byte(s, start_char);
    let be = char_to_byte(s, end_char);
    s.replace_range(bs..be, "");
}

pub fn ref_line_count(s: &str) -> usize {
    s.chars().filter(|&c| c == '\n').count() + 1
}

pub fn ref_line_col(s: &str, pos: usize) -> (usize, usize) {
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

pub fn ref_line_start(s: &str, line: usize) -> usize {
    if line == 0 {
        return 0;
    }
    let mut seen = 0;
    let mut off = 0;
    for c in s.chars() {
        off += 1;
        if c == '\n' {
            seen += 1;
            if seen == line {
                return off;
            }
        }
    }
    off
}

pub fn ref_line_slice(s: &str, line: usize) -> String {
    s.split('\n').nth(line).unwrap_or("").to_string()
}

pub fn ref_line_col_to_offset(s: &str, line: usize, col: usize) -> usize {
    let total = ref_char_len(s);
    let start = ref_line_start(s, line);
    let end = if line + 1 < ref_line_count(s) {
        ref_line_start(s, line + 1) - 1
    } else {
        total
    };
    (start + col).min(end)
}
