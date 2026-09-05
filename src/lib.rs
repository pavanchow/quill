//! Quill is a dependency-free editable text buffer.
//!
//! The crate has no external dependencies and targets the Rust 2021 edition. It
//! provides a piece table backed buffer with correct line and column indexing,
//! a cursor and selection model, a coalescing undo/redo history, and substring
//! search. The [`TextEditor`] type is the high level entry point; [`PieceTable`]
//! is available directly when only the buffer is needed.
//!
//! ```
//! use quill::TextEditor;
//!
//! let mut ed = TextEditor::new("hello");
//! ed.insert(5, " world");
//! assert_eq!(ed.contents(), "hello world");
//! assert_eq!(ed.line_col(11), (0, 11));
//! ed.undo();
//! assert_eq!(ed.contents(), "hello");
//! ```

pub mod cursor;
pub mod editor;
pub mod piece_table;
pub mod search;

pub use cursor::{Cursor, CursorSet};
pub use editor::TextEditor;
pub use piece_table::{PieceInfo, PieceTable};
pub use search::{count as search_count, find_all};
