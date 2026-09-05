//! Quill command line driver.
//!
//! This is a thin exerciser for the editor engine, not a full terminal UI. It
//! can open a file into the buffer, run a scripted set of edits, and print
//! statistics and a region. With no file it runs a self contained `demo` that
//! shows inserts, deletes, undo, redo, search, and the piece structure.

use quill::TextEditor;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.is_empty() || args[0] == "demo" {
        run_demo();
        return ExitCode::SUCCESS;
    }

    if args[0] == "--help" || args[0] == "-h" {
        print_usage();
        return ExitCode::SUCCESS;
    }

    let path = &args[0];
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("quill: cannot read {path}: {e}");
            return ExitCode::FAILURE;
        }
    };

    // Optional region: `--region <start_line> <end_line>` (zero based, inclusive).
    let mut region: Option<(usize, usize)> = None;
    let mut i = 1;
    while i < args.len() {
        if args[i] == "--region" && i + 2 < args.len() {
            let a = args[i + 1].parse::<usize>().unwrap_or(0);
            let b = args[i + 2].parse::<usize>().unwrap_or(a);
            region = Some((a, b));
            i += 3;
        } else {
            eprintln!("quill: unknown argument {}", args[i]);
            return ExitCode::FAILURE;
        }
    }

    run_on_file(path, &text, region);
    ExitCode::SUCCESS
}

fn print_usage() {
    println!("quill - dependency free text buffer engine");
    println!();
    println!("Usage:");
    println!("  quill                      run the demo");
    println!("  quill demo                 run the demo");
    println!("  quill <file>               open a file, run scripted edits, print stats");
    println!("  quill <file> --region A B  also print lines A..=B (zero based)");
}

fn print_stats(label: &str, ed: &TextEditor) {
    println!(
        "{label}: {} lines, {} chars, {} pieces",
        ed.line_count(),
        ed.char_len(),
        ed.buffer().piece_count()
    );
}

fn print_region(ed: &TextEditor, start: usize, end: usize) {
    let last = ed.line_count().saturating_sub(1);
    let end = end.min(last);
    println!("--- region lines {start}..={end} ---");
    for line in start..=end {
        println!("{:>4} | {}", line, ed.line(line));
    }
    println!("--- end region ---");
}

fn run_on_file(path: &str, text: &str, region: Option<(usize, usize)>) {
    let mut ed = TextEditor::new(text);
    println!("opened {path}");
    print_stats("before", &ed);

    // Scripted set of edits: tag the top, tag the end, and normalize a word.
    let banner = "== quill ==\n";
    ed.insert(0, banner);
    let end = ed.char_len();
    ed.insert(end, "\n== end ==\n");
    let matches = ed.find_all("the");
    if let Some(&first) = matches.first() {
        ed.replace(first, first + 3, "THE");
    }

    print_stats("after ", &ed);
    println!("found {} occurrence(s) of \"the\"", matches.len());

    let (a, b) = region.unwrap_or((0, 4));
    print_region(&ed, a, b);
}

fn run_demo() {
    println!("quill demo");
    println!("==========");

    let mut ed = TextEditor::new("The quick brown fox\n");
    print_stats("start", &ed);
    println!("text: {:?}", ed.contents());
    println!();

    // Insert at an interior offset.
    let at = ed.char_len() - 1; // just before the trailing newline
    ed.insert(at, " jumps");
    println!("after insert \" jumps\": {:?}", ed.contents());

    // Append a second line by typing character by character (coalesced undo).
    let base = ed.char_len();
    for (i, c) in "over the lazy dog".chars().enumerate() {
        ed.type_text(base + i, &c.to_string());
    }
    ed.commit();
    println!("after typing line 2: {:?}", ed.contents());
    print_stats("now  ", &ed);
    println!();

    // Line and column queries.
    let (l, c) = ed.line_col(4);
    println!("offset 4 is at line {l}, column {c}");
    println!("line 1 is: {:?}", ed.line(1));
    println!();

    // Search.
    let hits = ed.find_all("the");
    println!("search \"the\" matched at char offsets {hits:?}");
    println!();

    // Replace as one undo group.
    if let Some(&first) = hits.first() {
        ed.replace(first, first + 3, "THE");
        println!("after replace first \"the\" -> \"THE\": {:?}", ed.contents());
    }
    println!();

    // Undo and redo.
    ed.undo();
    println!("after undo (replace):   {:?}", ed.contents());
    ed.undo();
    println!("after undo (typed line):{:?}", ed.contents());
    ed.redo();
    println!("after redo (typed line):{:?}", ed.contents());
    ed.redo();
    println!("after redo (replace):   {:?}", ed.contents());
    println!();

    // Piece structure visualization.
    println!("piece table structure ({} pieces):", ed.buffer().piece_count());
    for (i, p) in ed.pieces().iter().enumerate() {
        println!(
            "  [{i}] {:<8} bytes {}..{} = {:?}",
            p.source,
            p.start,
            p.start + p.byte_len,
            p.text
        );
    }
    println!();
    print_stats("final", &ed);
}
