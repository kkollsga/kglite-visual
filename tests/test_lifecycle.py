"""L5 — the three shutdown paths, each tested in the process shape it exists
for.

Two of them cannot be tested in-process at all: "the interpreter exits without
anyone calling close()" needs a *whole interpreter* to exit, and "a fork
inherited the handle" needs a fork. Both run as subprocesses with a wall-clock
timeout, because the failure they guard against is a **hang**, and a hang is
invisible to any assertion that runs after it.
"""

from __future__ import annotations

import os
import subprocess
import sys
import textwrap

import pytest

from conftest import REPO_ROOT, port_is_listening

TIMEOUT = 90


def run_child(body: str, cwd=None) -> subprocess.CompletedProcess:
    script = textwrap.dedent(body)
    try:
        return subprocess.run(
            [sys.executable, "-c", script],
            capture_output=True,
            text=True,
            timeout=TIMEOUT,
            cwd=str(cwd or REPO_ROOT),
        )
    except subprocess.TimeoutExpired as expired:
        pytest.fail(
            f"the child process hung for {TIMEOUT}s — the shutdown path under "
            f"test does not terminate.\nstdout so far: {expired.stdout!r}"
        )


def test_a_process_that_never_calls_close_still_exits(fixture_path):
    """The atexit hook. Without it, `python -c "... show(g)"` never returns:
    the server thread is alive and nothing has asked it to stop."""
    result = run_child(
        f"""
        import kglite_visual as kv
        view = kv.show({fixture_path!r}, open_browser=False)
        print(view.port, flush=True)
        # No close(), deliberately.
        """
    )
    assert result.returncode == 0, result.stderr
    port = int(result.stdout.strip())
    assert not port_is_listening(port), "the port outlived the process"


def test_the_wheel_writes_nothing_to_stdout(fixture_path):
    """A library that prints to stdout corrupts whatever its caller was
    writing there. The CLI owns the single-JSON-line contract; the wheel
    returns the same data instead."""
    result = run_child(
        f"""
        import kglite_visual as kv
        view = kv.show({fixture_path!r}, open_browser=False)
        view.close()
        """
    )
    assert result.returncode == 0, result.stderr
    assert result.stdout == "", f"unexpected stdout: {result.stdout!r}"


@pytest.mark.skipif(not hasattr(os, "fork"), reason="no fork() on this platform")
def test_a_forked_child_refuses_to_close_its_parents_server(fixture_path):
    """The PID guard. `fork()` copies the handle but not the server thread, so
    a child that tried to honour it would join a thread its process never
    started — and would report a server stopped that is still serving."""
    result = run_child(
        f"""
        import os, sys, warnings
        import kglite_visual as kv
        view = kv.show({fixture_path!r}, open_browser=False)
        read_fd, write_fd = os.pipe()
        pid = os.fork()
        if pid == 0:
            os.close(read_fd)
            try:
                with warnings.catch_warnings(record=True) as caught:
                    warnings.simplefilter("always")
                    status = view.close()
                message = str(caught[0].message) if caught else ""
                os.write(write_fd, (status + "|" + message).encode())
            finally:
                os._exit(0)
        os.close(write_fd)
        os.waitpid(pid, 0)
        with os.fdopen(read_fd, "rb") as pipe:
            status, _, message = pipe.read().decode().partition("|")
        print("CHILD_STATUS=" + status)
        print("CHILD_WARNED=" + str("forked process" in message))
        # The parent's server is untouched by any of that.
        import http.client
        conn = http.client.HTTPConnection("127.0.0.1", view.port, timeout=5)
        conn.request("GET", "/api/session")
        print("PARENT_STILL_SERVING=" + str(conn.getresponse().status == 200))
        conn.close()
        view.close()
        """
    )
    assert result.returncode == 0, result.stderr
    assert "CHILD_STATUS=stale-after-fork" in result.stdout, result.stdout
    assert "CHILD_WARNED=True" in result.stdout, result.stdout
    assert "PARENT_STILL_SERVING=True" in result.stdout, result.stdout


def test_a_dropped_handle_frees_its_port(fixture_path):
    """The garbage-collection path: no close(), no reference, no atexit — the
    native handle's own Drop has to signal the server."""
    import gc

    import kglite_visual as kv

    view = kv.show(fixture_path, open_browser=False)
    port = view.port
    assert port_is_listening(port)
    del view
    gc.collect()
    for _ in range(100):
        if not port_is_listening(port, timeout=0.1):
            break
    assert not port_is_listening(port), "a collected handle leaked its port"


def test_the_console_script_is_the_cli(fixture_path):
    """The lib-link (plan D9): the wheel's `kglite-visual` command is the same
    program as the binary, including the single-JSON-line stdout contract."""
    import json

    script = REPO_ROOT / ".venv" / "bin" / "kglite-visual"
    if not script.exists():  # pragma: no cover - only when run outside `make pytest`
        pytest.skip(f"console script not installed at {script}")
    process = subprocess.Popen(
        [str(script), fixture_path, "--no-open", "--port", "0"],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    try:
        line = process.stdout.readline()
        info = json.loads(line)
        assert sorted(info) == ["graph", "mcp", "pid", "port", "url"]
        assert info["port"] > 0
        assert port_is_listening(info["port"])
    finally:
        process.terminate()
        process.wait(timeout=30)
