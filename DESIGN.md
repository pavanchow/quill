# Quill design

This document explains how Quill is built, the data structure at its core, how
line indexing stays correct, how undo and redo work, and why each correctness
gate proves what it claims. No dependencies are used anywhere. The code targets
the Rust 2021 edition and the standard library only.

## Architecture

The crate is a small set of layers, each in its own module.

- `piece_table` is the buffer. It owns the text and every content operation.
- `cursor` is the caret and selection model, independent of the buffer.
- `search` is substring search over a text snapshot.
- `editor` composes the buffer, cursors, and an undo and redo history into a
  single editable document, the `TextEditor` type most callers use.
- `bin/quill.rs` is a command line driver that exercises the engine.

The buffer is the deliverable, so it is the layer with the differential proof.

## Why a piece table

A text buffer has to make repeated edits cheap on large documents. A plain
`String` fails this because inserting or deleting near the front shifts every
byte after the edit point, so each edit is linear in the size of the document.

Two classic structures fix this, the rope and the piece table. A rope is a
balanced tree of text chunks. It has excellent asymptotics but the balancing
logic is intricate and easy to get subtly wrong. A piece table is simpler and
maps naturally onto how editors actually work, so it was the chosen structure
here. Simplicity matters because the whole point of Quill is a buffer you can
read and trust.

A piece table keeps two immutable text stores. The `original` store holds the
text the buffer was opened with. The `add` store is append only and receives
every inserted string. The visible document is a list of pieces, where each
piece points at a byte range inside one of the two stores. Because the stores
are immutable and the add store only grows, text is never rewritten in place.

- Insert appends the new text to the add store and splices at most one new piece
  into the list, splitting the piece at the insertion point if needed.
- Delete splits at the two range ends and drops the pieces in between.
- Replace is a delete followed by an insert.

The cost of an edit is proportional to the number of pieces touched, not the
length of the document, which is the property a `String` lacks. A contiguous run
of appends at the end of the buffer is coalesced into a single growing piece, so
ordinary typing does not fragment the table.

A delete leaves a seam where the removed pieces were drained. If the two pieces
that now touch come from the same store and are byte contiguous, they are merged
back into one piece in constant time, combining the cached counters from the two
halves without rescanning any text. This matters because steady state editing,
typing a few characters and deleting them again at scattered positions, would
otherwise leave one permanent piece behind per cycle. With the seam merge the
piece count under such churn stays flat, which the stress gate asserts directly.
The structural size of the table therefore tracks the edit count, never the
volume of the document, and a same-size gate test on a one megabyte and a four
megabyte document produces the same piece count for the same edit script.

Every public offset is a character offset, a count of Unicode scalar values, not
a byte offset. A character offset can never point inside a multibyte UTF-8
sequence, so every edit position is valid and no edit can split a code point.
Each piece caches its character length, its newline count, and the number of
characters after its last newline, which makes the length, line, and column
queries fast without rescanning the whole document.

## Line indexing

Line and column information has to stay correct across arbitrary edits, and it
has to be cheap to query.

The total line count is maintained incrementally. The buffer tracks the total
number of newline characters, and the line count is that total plus one. An
empty buffer is one line, and a trailing newline produces a final empty line,
which matches how editors count lines.

Mapping a character offset to a line and column walks the piece list. Pieces
that lie entirely before the offset are skipped using their cached newline count
and trailing character count, so whole pieces are consumed in constant time. Only
the piece that contains the offset is scanned character by character. Mapping the
other way, from a line to its starting offset, walks pieces counting newlines
until the target line begins. A line and column pair is turned into an offset by
finding the line start and adding the column, clamped to the end of that line so
an out of range column never escapes its line. Reading a single line slices
between its start and the start of the next line minus the newline.

All of these are derived from the same piece metadata that the edit operations
maintain, so they cannot drift out of sync with the contents.

## Undo and redo model

History lives in the `editor` layer, not the buffer, so the buffer stays a pure
data structure.

Each content change is recorded as one or more primitive, invertible operations.
An insert records the text and where it went, so its inverse is a delete of that
range. A delete records the text it removed and where, so its inverse is an
insert of that text. A replace records a delete followed by an insert.

Primitives are grouped into transactions, and a transaction is the unit of undo
and redo. Explicit `insert`, `delete`, and `replace` each form their own
transaction. Typing through `type_text` coalesces: consecutive single character
inserts at the caret extend the current transaction instead of starting a new
one, so a single undo removes a whole typed word rather than one letter at a
time. Any delete, any non contiguous edit, or an explicit `commit` closes the
current transaction.

Undo pops a transaction, applies the inverse of each primitive in reverse order,
and pushes the transaction onto the redo stack. Redo does the reverse. Making any
new edit clears the redo stack, so a redo is only ever valid until the next
change, which is the behavior users expect. Undo past the beginning reports
false and leaves the buffer untouched.

## Multi-cursor editing and structured replace

The cursor set already held multiple carets. The editor now edits through them
as a unit.

`insert_at_cursors` inserts the same text at every caret, replacing the
selection of any cursor that has one. The cursors are applied from the highest
offset down, so lower offsets stay valid while higher ones are edited. Each
cursor's edit is recorded as one transaction of its own, so undo walks back one
cursor at a time and the per-cursor grouping is directly observable. After the
edit every caret sits just past its own copy of the text, shifted by the
insertions of lower cursors.

`delete_selections` removes every selected range under the same discipline,
highest offset first and one transaction per deletion. Cursors that deleted
collapse to their selection starts, bare carets stay put, and everything is
shifted by the deletions below it.

Structured replace is built on the same transaction primitives.
`replace_all` finds every non overlapping occurrence left to right on a
snapshot of the text, applies the replacements from the highest offset down as
one transaction, and returns the match count. Because the occurrences come from
one pass over the pre-edit text, a replacement that contains the needle is never
rescanned. `replace_next` replaces the first occurrence at or after an offset as
its own transaction and returns the range covering the inserted replacement, so
an interactive caller can continue searching past it one step at a time.

The gate composes the same edits as single-cursor splices on a plain `String`
and requires the multi-cursor result to match, checks that undo count equals
cursor count with intermediate states matching the oracle, and pins replace-all
against a naive character scan.

## Stress scaling

Beyond the bounded gates, `tests/stress.rs` runs the same properties at
environment scaled sizes with small CI defaults. The mixed differential drives
hundreds of thousands of edits into multi-megabyte documents against a
`Vec<char>` oracle whose splices cost the window rather than the document, with
scalar counts checked every operation and full contents compared at checkpoints.
The churn phase interleaves bounded undo and redo walks with fresh edits and
performs exact per-burst undo and redo round trips. The boundary phase edits at
every character offset and every byte offset, including offsets inside multibyte
sequences, of a dense multibyte document. The search phase compares matches
against a naive finder while the document is being edited. The knobs are
`QUILL_FUZZ_OPS`, `QUILL_STRESS_SEEDS`, and `QUILL_STRESS_DOC_KB`.

## Why each gate proves its claim

The gates are committed as tests and are bounded for CI, with the operation
count set by `QUILL_FUZZ_OPS`.

The differential gate proves the buffer is a correct text container and a correct
line and column index. It runs the identical random stream of inserts, deletes,
and replaces against the piece table and against a plain `String` whose behavior
is obviously correct by construction. After every single operation it asserts the
full contents are equal and that every query agrees with the reference computed
from the `String`: total length, line count, offset to line and column at sampled
offsets, line start offset, line slice for every line, and line and column back
to an offset. Because the reference is trivially correct and the check runs after
every operation, any divergence in the piece table is caught immediately, which
is exactly the claim that the buffer and its indexing are correct across edits.

The undo and redo gate proves the history is sound. From an initial buffer it
applies a random op sequence, undoes everything, and asserts the buffer equals
the initial state exactly, then redoes everything and asserts it equals the post
edit state. It also records the state after every operation and then performs a
random walk of undo and redo, asserting the buffer matches the recorded state at
each step. This proves undo and redo are true inverses and that mixed sequences
stay consistent, not just a single undo of a single edit.

The invariants gate proves safety and the cheap counters. Over a random op stream
it asserts the character length, byte length, and line count always match the
reference, so the incrementally maintained totals never drift. It then edits at
every kind of boundary, the start, the end, an empty buffer, and every character
offset across a string of multibyte characters, asserting no panic and correct
lengths. This proves the buffer is robust at the edges where off by one and UTF-8
boundary bugs usually live.
