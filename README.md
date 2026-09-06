# Quill

Quill is a dependency-free editable text buffer written from scratch in Rust. It
gives you a piece table backed document with correct line and column indexing, a
cursor and selection model, a coalescing undo and redo history, and substring
search. No external crates, pure standard library, Rust 2021 edition.

Live playground: https://pavanchow.github.io/quill/

## What it is

Most editor buffers are buried inside a large application or pull in a tree of
dependencies. Quill is just the buffer, as a clean library you can drop into any
project. The provable core is the text buffer itself, verified against a naive
reference on every edit.

## The gap it fills

A plain `String` is fine until you need to edit a large document repeatedly.
Inserting a character near the front of a megabyte of text rewrites the whole
tail every time. Quill uses a piece table, so an edit costs work proportional to
the pieces it touches, not the size of the document. On top of that it keeps line
and column information correct across every edit, which a raw `String` does not.

A person building a small editor, a code formatter, a REPL, a notes tool, or a
diff viewer needs exactly this and nothing more. An AI agent that edits text or
source files needs a buffer it can reason about with stable offsets, line and
column mapping, and real undo, without dragging in a dependency graph it cannot
audit. Quill is small enough to read end to end and trust.

## Quickstart

```bash
cargo build --release
cargo test
./target/release/quill demo
./target/release/quill path/to/file.txt --region 0 10
```

Library usage:

```rust
use quill::TextEditor;

let mut ed = TextEditor::new("hello");
ed.insert(5, " world");            // "hello world"
assert_eq!(ed.line_col(11), (0, 11));

ed.replace(0, 5, "HELLO");         // "HELLO world"
ed.undo();                         // back to "hello world"
ed.redo();                         // "HELLO world"

let hits = ed.find_all("o");       // char offsets of every "o"
```

All offsets are character offsets (counts of Unicode scalar values), so no edit
can ever fall inside a multibyte UTF-8 sequence.

## API

`TextEditor` is the high level entry point.

- `new(text)`, `empty()` build a document.
- `insert(pos, text)`, `delete(start, end)`, `replace(start, end, text)` edit it.
- `type_text(pos, text)` inserts and coalesces adjacent typing into one undo group.
- `insert_at_cursors(text)` inserts at every caret or replaces every selection, one undo step per cursor.
- `delete_selections()` removes every selection, one undo step per cursor.
- `replace_all(needle, text)` rewrites every occurrence in a single undo group.
- `replace_next(needle, text, from)` replaces one occurrence for interactive flows.
- `undo()`, `redo()`, `commit()`, `can_undo()`, `can_redo()` drive history.
- `contents()`, `char_len()`, `line_count()`, `is_empty()` read the document.
- `line_col(pos)`, `line(n)` map offsets and read lines.
- `find_all(needle)` returns the character offset of every match.
- `pieces()` returns a structural view for visualization.
- `cursors()`, `set_cursors(..)` manage the cursor and selection set.

`PieceTable` is the buffer by itself when you do not need history, with
`insert`, `delete`, `replace`, `slice`, `contents`, `offset_to_line_col`,
`line_start_offset`, `line_col_to_offset`, `line_slice`, and `piece_view`.

`Cursor` and `CursorSet` model carets and selections, including multiple cursors.

`find_all` and `search_count` expose search directly.

## The correctness gate

The buffer is proved by tests committed with the crate. They are bounded for CI
and the operation count is controllable with `QUILL_FUZZ_OPS`.

```bash
QUILL_FUZZ_OPS=20000 cargo test --release
```

1. Differential against a naive `String` buffer. The same random stream of
   insert, delete, and replace operations is applied to the piece table and to a
   plain `String`. After every operation the full contents must match, and so
   must every line and column query.
2. Undo and redo round-trip. A random op sequence is fully undone back to the
   exact initial state, then fully redone to the post edit state, and a mixed
   walk of undo and redo is checked against a reference list of every state.
   Undo past the beginning stops cleanly, a new edit kills redo, and whole
   document replaces round trip as one group.
3. Buffer invariants. Total length and line count stay correct, and no edit
   panics at a boundary: start, end, empty buffer, or a multibyte UTF-8 boundary.
4. Multi-cursor and structured replace. Multi-cursor inserts, selection
   replacement, and selection deletion are compared against the same edits
   composed as single-cursor `String` splices, undo grouping is checked per
   cursor, replace-all is pinned to a naive character scan, and piece count is
   asserted to track the edit count rather than the document size.

The stress harness in `tests/stress.rs` scales the same properties to millions
of operations and multi-megabyte documents: a mixed edit stream against a
`Vec<char>` oracle with checkpointed full comparisons, undo and redo churn with
exact per-burst round trips, an edit at every character and byte offset of a
multibyte document, and search edge cases at scale. It shares the small CI
defaults and the same env knobs.

## Design

See DESIGN.md for the architecture, the choice of a piece table, line indexing,
the undo and redo model, and why each gate proves what it claims.

## License

MIT.
