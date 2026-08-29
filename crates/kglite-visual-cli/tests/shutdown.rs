//! Stopping the server, driven as a process.
//!
//! **A signal is only observable from outside.** `SIGTERM`'s default
//! disposition terminates the process without running a single destructor, so
//! whether this binary cleans up after itself is a fact about the *process*,
//! invisible to any in-process test. This one starts the real binary, kills it
//! the way a supervisor or a shell does, and looks at what it left behind.
//!
//! Unix only: there is no `SIGTERM` to send on Windows, and the Windows build
//! catches Ctrl-C instead, which a test harness cannot deliver to one child.
#![cfg(unix)]

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

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

/// A private `TMPDIR` for one test, so the spill this test is about cannot be
/// confused with one some other process left in the shared directory.
fn private_tmpdir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "kglv-shutdown-{}-{}",
        std::process::id(),
        Instant::now().elapsed().as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("a private TMPDIR");
    dir
}

/// kglite spills a portable graph into `$TMPDIR/kglite_portable_<pid>_<id>/`
/// and removes it in `Drop`. Its presence is what makes this test non-vacuous:
/// a fixture too small to spill would let a broken handler pass on an empty
/// directory.
fn spills(tmpdir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(tmpdir)
        .expect("the private TMPDIR is readable")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("kglite_portable_"))
        })
        .collect()
}

struct Launched {
    child: Child,
    port: u16,
}

fn launch(tmpdir: &Path) -> Launched {
    let mut child = Command::new(binary())
        .arg(fixture())
        .args(["--no-open", "--port", "0"])
        .env("TMPDIR", tmpdir)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("the viewer starts");

    // The launch contract: one JSON line on stdout, printed after the socket is
    // listening. Reading it is also how this test avoids racing the server it
    // is about to kill.
    let mut line = String::new();
    BufReader::new(child.stdout.take().expect("stdout is piped"))
        .read_line(&mut line)
        .expect("the launch line arrives");
    let port = line
        .split("\"port\":")
        .nth(1)
        .and_then(|rest| rest.split(&[',', '}'][..]).next())
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or_else(|| panic!("no port in the launch line: {line:?}"));

    Launched { child, port }
}

/// `SIGTERM` stops the server, and stopping it removes kglite's spill.
///
/// **The failure this replaces had a directory count.** With no handler
/// installed the default disposition terminates the process, `Drop` never runs,
/// and the spilled graph stays — one directory per stopped server, and this
/// project's own working folder had fifty of them before anyone counted. The
/// exit code is part of the contract too: a process that dies *from* the signal
/// reports no code at all, which is how a supervisor learns it did not stop
/// cleanly.
#[test]
fn sigterm_stops_the_server_and_takes_the_spill_with_it() {
    let tmpdir = private_tmpdir();
    let launched = launch(&tmpdir);
    let mut child = launched.child;

    assert!(
        !spills(&tmpdir).is_empty(),
        "the fixture did not spill, so this test would pass on an empty directory"
    );

    let killed = Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status()
        .expect("kill runs");
    assert!(killed.success(), "the signal was delivered");

    let status = wait_briefly(&mut child);
    assert_eq!(
        status.code(),
        Some(0),
        "a caught SIGTERM is a clean stop; `None` means the process died from \
         the signal with no destructor run"
    );
    assert!(
        spills(&tmpdir).is_empty(),
        "kglite's spill survived the shutdown: {:?}",
        spills(&tmpdir)
    );

    // And the port is back, which is the other half of what a caller who
    // stopped the server is waiting for.
    std::net::TcpListener::bind(("127.0.0.1", launched.port))
        .expect("the listener was closed and the port released");

    let _ = std::fs::remove_dir_all(&tmpdir);
}

/// Wait for the child, but never forever: a handler that swallows the signal
/// without stopping would hang the suite rather than fail it.
fn wait_briefly(child: &mut Child) -> std::process::ExitStatus {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        match child.try_wait().expect("the child is waitable") {
            Some(status) => return status,
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                panic!("the server did not stop within 20s of SIGTERM");
            }
            None => std::thread::sleep(Duration::from_millis(50)),
        }
    }
}
