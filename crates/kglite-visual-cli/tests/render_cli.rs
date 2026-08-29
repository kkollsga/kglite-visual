//! The render subcommand, driven as a process.
//!
//! **Driven, not called.** The launch contract is about what a *process* puts on
//! stdout and what it exits with, and neither is observable from inside the
//! library: an in-process test cannot see that a stray `println!` broke the one
//! line, and it cannot see the exit code at all. So this spawns the real binary
//! the way an agent would.

use std::path::{Path, PathBuf};
use std::process::Command;

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_kglite-visual")
}

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("kglite-visual-core")
        .join("tests")
        .join("fixtures")
        .join("meta.kgl")
}

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

fn run(args: &[&str]) -> Run {
    let output = Command::new(binary())
        .args(args)
        .output()
        .expect("the render binary runs");
    Run {
        code: output.status.code().expect("the process exited normally"),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    }
}

#[test]
fn a_meta_render_writes_the_file_and_prints_exactly_one_json_line() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let out = dir.path().join("meta.svg");
    let run = run(&[
        "render",
        &fixture().display().to_string(),
        "--meta",
        "--out",
        &out.display().to_string(),
        "--seed",
        "1234",
    ]);

    assert_eq!(run.code, 0, "stderr was: {}", run.stderr);
    assert_eq!(
        run.stdout.lines().count(),
        1,
        "the stdout contract is exactly one line, got: {:?}",
        run.stdout
    );

    let summary: serde_json::Value =
        serde_json::from_str(run.stdout.trim()).expect("the one line is JSON");
    assert_eq!(summary["out"], out.display().to_string());
    assert_eq!(summary["format"], "svg");
    assert_eq!(summary["nodes"], 5, "the fixture has five types");
    assert_eq!(summary["truncated"], false);
    assert!(summary["bytes"].as_u64().expect("a byte count") > 0);

    let written = std::fs::read_to_string(&out).expect("the file the line names exists");
    assert_eq!(
        written.len(),
        summary["bytes"].as_u64().unwrap() as usize,
        "the reported byte count is the file's"
    );
    assert!(written.starts_with("<?xml"), "an SVG document");
}

/// The error path an agent has to be able to distinguish from an empty answer.
///
/// Three assertions, and each is a separate way this can go wrong: a zero exit
/// on a failed render, a half-written summary line on stdout that a harness
/// would parse as success, and a message that throws away kglite's own
/// diagnostic — which is the only part naming the position in the query.
#[test]
fn a_bad_cypher_exits_one_with_empty_stdout_and_the_engines_own_message() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let run = run(&[
        "render",
        &fixture().display().to_string(),
        "--cypher",
        "MATCH (n RETURN n",
        "--out",
        &dir.path().join("bad.svg").display().to_string(),
    ]);

    assert_eq!(run.code, 1, "a failed render is a failed process");
    assert!(
        run.stdout.is_empty(),
        "stdout must stay empty on failure; a harness reads a line as success: {:?}",
        run.stdout
    );
    assert!(
        run.stderr.contains("kglite-visual:"),
        "the diagnostic is prefixed like every other one: {:?}",
        run.stderr
    );
    assert!(
        run.stderr.len() > "kglite-visual: query failed".len(),
        "the engine's own message carries the position and the expected token; \
         replacing it with a generic line throws away the only actionable part: {:?}",
        run.stderr
    );
    assert!(
        !dir.path().join("bad.svg").exists(),
        "a failed render must not leave a file behind for something else to read"
    );
}

#[test]
fn a_missing_graph_file_fails_before_anything_is_written() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let out = dir.path().join("nope.svg");
    let run = run(&[
        "render",
        &dir.path().join("does-not-exist.kgl").display().to_string(),
        "--meta",
        "--out",
        &out.display().to_string(),
    ]);
    assert_eq!(run.code, 1);
    assert!(run.stdout.is_empty());
    assert!(!out.exists());
}

/// A type the graph does not have is a request error, not a blank picture.
#[test]
fn an_unknown_expand_type_is_refused_by_name() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let run = run(&[
        "render",
        &fixture().display().to_string(),
        "--expand",
        "type=Wellbore",
        "--out",
        &dir.path().join("x.svg").display().to_string(),
    ]);
    assert_eq!(run.code, 1);
    assert!(run.stdout.is_empty());
    assert!(run.stderr.contains("Wellbore"), "{}", run.stderr);
}

/// The same request twice is the same file, byte for byte.
///
/// The determinism claim, made where it is actually consumed: two *processes*,
/// so nothing shared in memory can be doing the work. A layout that depended on
/// a hash-map order, an address, or a clock would pass every in-process test and
/// fail here.
#[test]
fn two_processes_rendering_one_request_write_identical_bytes() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let mut written: Vec<Vec<u8>> = Vec::new();
    for name in ["a.svg", "b.svg"] {
        let out = dir.path().join(name);
        let run = run(&[
            "render",
            &fixture().display().to_string(),
            "--cypher",
            "MATCH (p:Person)-[r:WORKS_AT]->(c:Company) RETURN p, r, c",
            "--limit",
            "24",
            "--out",
            &out.display().to_string(),
            "--seed",
            "7",
        ]);
        assert_eq!(run.code, 0, "stderr: {}", run.stderr);
        written.push(std::fs::read(&out).expect("the render wrote a file"));
    }
    assert_eq!(written[0], written[1]);
}

/// Adding a subcommand to a parser whose first positional was required is
/// exactly how a working `kglite-visual g.kgl` becomes a usage error. `--help`
/// is the cheapest observation of both forms still existing.
#[test]
fn the_bare_serve_form_and_the_render_subcommand_both_still_parse() {
    let help = run(&["--help"]);
    assert_eq!(help.code, 0);
    assert!(help.stdout.contains("render"), "{}", help.stdout);

    let render_help = run(&["render", "--help"]);
    assert_eq!(render_help.code, 0);
    for flag in [
        "--cypher", "--meta", "--expand", "--format", "--seed", "--out",
    ] {
        assert!(
            render_help.stdout.contains(flag),
            "{flag} is missing from the subcommand's help:\n{}",
            render_help.stdout
        );
    }

    // The bare form still wants a file and still says so with clap's usage
    // text and clap's exit code. `subcommand_negates_reqs` makes the positional
    // optional at the parser level, so without the guard in `run_from` this
    // invocation would parse and fail later with a different code and a
    // one-line message — a silent change to what a harness sees.
    let bare = run(&[]);
    assert_eq!(
        bare.code, 2,
        "clap's usage-error code, not a runtime failure"
    );
    assert!(bare.stderr.contains("<FILE>"), "{}", bare.stderr);
    assert!(bare.stderr.contains("Usage:"), "{}", bare.stderr);
}
