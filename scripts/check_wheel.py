#!/usr/bin/env python3
"""The packaged-consumer check: make the built wheel prove itself.

Every other test in this repo runs against the source tree, and there is one
class of defect a source-tree test structurally cannot see — **the artifact is
missing something the sources have**. For this project that is concrete: the
frontend bundle is baked into the extension module by ``rust-embed``, so a
wheel built from a tree with an empty ``frontend/dist`` imports fine, starts a
server fine, and answers ``/`` with an error page. Nothing upstream of the
wheel notices.

Two depths, because each misses what the other catches:

**Inventory** — open the ``.whl`` (it is a zip) and assert what is inside it:
one compiled extension, the Python shim, the console-script entry point, and —
by scanning the extension's own bytes — the *hashed asset names and content*
that ``frontend/dist/index.html`` says the bundle contains. A zip listing
cannot answer that last one: the bundle is inside the ``.so``, not beside it.

**Consumer** — install that wheel into a **fresh** virtualenv and run a probe
**outside the repo root**, so the source tree cannot shadow the package. The
probe starts a server on the committed fixture, fetches ``/api/session`` and
``/``, and checks it got the app rather than the no-bundle-embedded message.
Running it from inside the repo would prove nothing: ``python -c "import
kglite_visual"`` there resolves the editable install every time.

Exit codes: 0 clean, 1 the artifact is wrong, 2 the checker could not do its
job (no wheel, no fixture, no network-free venv).
"""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
WHEEL_DIR = ROOT / "target" / "wheels"
DIST_INDEX = ROOT / "frontend" / "dist" / "index.html"
FIXTURE = ROOT / "crates/kglite-visual-core/tests/fixtures/meta.kgl"

#: Every Python module the wheel must carry. A shim that shipped without
#: ``_notebook`` would import and then fail at the first ``_repr_html_``.
REQUIRED_MODULES = (
    "kglite_visual/__init__.py",
    "kglite_visual/_server.py",
    "kglite_visual/_notebook.py",
)

#: Vite emits hashed names; this pulls them out of the built index.html so the
#: check moves with the bundle instead of pinning a hash that changes on every
#: frontend edit.
ASSET_RE = re.compile(r'(?:src|href)="\./(assets/[^"]+)"')

#: A distinctive slice of the app's own index.html. Its presence in the
#: extension proves the *contents* were embedded, not merely the filenames.
INDEX_MARKER = b'<div id="app"></div>'

PROBE = '''\
"""Runs from a temp directory, in a venv holding only the built wheel."""
import http.client
import json
import socket
import sys

import kglite_visual as kv

view = kv.show(sys.argv[1], open_browser=False)
result = {"launch_info": view.launch_info, "version": kv.__version__}

conn = http.client.HTTPConnection("127.0.0.1", view.port, timeout=10)
conn.request("GET", "/api/session")
response = conn.getresponse()
result["session_status"] = response.status
result["session"] = json.loads(response.read())
conn.close()

conn = http.client.HTTPConnection("127.0.0.1", view.port, timeout=10)
conn.request("GET", "/")
response = conn.getresponse()
body = response.read()
conn.close()
result["index_status"] = response.status
result["index_bytes"] = len(body)
result["serves_the_app"] = b'<div id="app"></div>' in body
result["no_bundle_warning"] = b"no frontend bundle is embedded" in body

port = view.port
result["close"] = view.close()
probe = socket.socket()
probe.settimeout(2)
try:
    probe.connect(("127.0.0.1", port))
    result["port_freed"] = False
except OSError:
    result["port_freed"] = True
finally:
    probe.close()

print(json.dumps(result))
'''


def newest_wheel(directory: Path) -> Path | None:
    wheels = sorted(directory.glob("*.whl"), key=lambda p: p.stat().st_mtime)
    return wheels[-1] if wheels else None


def expected_assets() -> list[str]:
    """Hashed asset paths the built index.html references."""
    if not DIST_INDEX.is_file():
        return []
    return ASSET_RE.findall(DIST_INDEX.read_text(encoding="utf-8"))


def check_inventory(wheel: Path, assets: list[str]) -> list[str]:
    """What must be inside the wheel. Returns a list of problems."""
    problems: list[str] = []
    with zipfile.ZipFile(wheel) as archive:
        names = archive.namelist()

        extensions = [
            n
            for n in names
            if n.startswith("kglite_visual/_native")
            and n.endswith((".so", ".pyd", ".dylib"))
        ]
        if len(extensions) != 1:
            problems.append(
                f"expected exactly one compiled extension at "
                f"kglite_visual/_native*, found {extensions or 'none'}"
            )

        for module in REQUIRED_MODULES:
            if module not in names:
                problems.append(f"missing Python module: {module}")

        entry_points = [n for n in names if n.endswith("dist-info/entry_points.txt")]
        if not entry_points:
            problems.append("no entry_points.txt — the console script is not declared")
        else:
            text = archive.read(entry_points[0]).decode("utf-8")
            if "kglite-visual" not in text:
                problems.append(
                    f"entry_points.txt does not declare the kglite-visual "
                    f"console script: {text!r}"
                )

        # The half a zip listing cannot see: the bundle lives inside the
        # extension module, so the extension's own bytes are the evidence.
        if not assets:
            problems.append(
                f"no hashed asset names found in {DIST_INDEX} — the asset check "
                "would pass vacuously, which is not a pass"
            )
        elif extensions:
            blob = archive.read(extensions[0])
            for asset in assets:
                if asset.encode() not in blob:
                    problems.append(
                        f"{asset} is referenced by the built index.html but is "
                        f"not embedded in {extensions[0]} — this wheel was built "
                        "against a stale or empty frontend/dist"
                    )
            if INDEX_MARKER not in blob:
                problems.append(
                    f"{INDEX_MARKER.decode()} is not in {extensions[0]} — the "
                    "asset *names* may be embedded but the index.html body is not"
                )
    return problems


def check_consumer(wheel: Path, keep: bool = False) -> tuple[list[str], dict | None]:
    """Install the wheel into a fresh venv and drive it from outside the repo."""
    if not FIXTURE.is_file():
        return [f"missing fixture {FIXTURE}; the probe has nothing to serve"], None

    workdir = Path(tempfile.mkdtemp(prefix="kglv-consumer-"))
    try:
        venv = workdir / "venv"
        subprocess.run(
            [sys.executable, "-m", "venv", str(venv)],
            check=True,
            capture_output=True,
        )
        python = venv / "bin" / "python"
        if not python.exists():  # Windows layout
            python = venv / "Scripts" / "python.exe"

        install = subprocess.run(
            [
                str(python),
                "-m",
                "pip",
                "install",
                "--quiet",
                "--no-index",
                "--no-deps",
                str(wheel),
            ],
            capture_output=True,
            text=True,
        )
        if install.returncode != 0:
            return [f"pip install {wheel.name} failed:\n{install.stderr}"], None

        # Everything the probe touches is copied out of the repo, so nothing it
        # resolves can come from the source tree.
        probe_path = workdir / "probe.py"
        probe_path.write_text(PROBE, encoding="utf-8")
        fixture_copy = workdir / FIXTURE.name
        shutil.copy2(FIXTURE, fixture_copy)

        env = dict(os.environ)
        env.pop("PYTHONPATH", None)
        run = subprocess.run(
            [str(python), str(probe_path), str(fixture_copy)],
            cwd=str(workdir),
            capture_output=True,
            text=True,
            timeout=180,
            env=env,
        )
        if run.returncode != 0:
            return [
                f"the packaged consumer probe failed (exit {run.returncode}):\n"
                f"{run.stdout}\n{run.stderr}"
            ], None

        report = json.loads(run.stdout.strip().splitlines()[-1])
        problems = []
        if report["session_status"] != 200:
            problems.append(f"/api/session returned {report['session_status']}")
        if not report["serves_the_app"]:
            problems.append("/ did not serve the app's index.html")
        if report["no_bundle_warning"]:
            problems.append(
                "the installed wheel says no frontend bundle is embedded in it"
            )
        if not report["port_freed"]:
            problems.append("close() did not release the port")
        if report["launch_info"]["port"] <= 0:
            problems.append("the launch contract reported an unresolved port")
        return problems, report
    finally:
        if not keep:
            shutil.rmtree(workdir, ignore_errors=True)


def check(wheel: Path, skip_consumer: bool = False) -> int:
    assets = expected_assets()
    problems = check_inventory(wheel, assets)
    report = None
    if not problems and not skip_consumer:
        consumer_problems, report = check_consumer(wheel)
        problems += consumer_problems

    if problems:
        print(f"check-wheel: FAIL — {wheel.name}", file=sys.stderr)
        for problem in problems:
            print(f"  {problem}", file=sys.stderr)
        return 1

    print(f"check-wheel: OK — {wheel.name} ({wheel.stat().st_size / 1e6:.1f} MB)")
    print(f"  embedded assets verified: {', '.join(assets)}")
    if report:
        info = report["launch_info"]
        print(
            f"  packaged consumer: served {info['graph']} on port {info['port']}, "
            f"/api/session 200, index {report['index_bytes']} B, "
            f"close -> {report['close']}, port freed"
        )
    return 0


def _doctor(wheel: Path, out: Path, drop: tuple[str, ...] = (), stub_so: bool = False,
            shim: str | None = None) -> Path:
    """Copy a wheel, breaking exactly one thing. The self-test's instrument."""
    with zipfile.ZipFile(wheel) as source, zipfile.ZipFile(out, "w") as target:
        for item in source.infolist():
            if any(item.filename.startswith(prefix) for prefix in drop):
                continue
            data = source.read(item.filename)
            if stub_so and item.filename.startswith("kglite_visual/_native"):
                data = b"not really an extension module"
            if shim is not None and item.filename == "kglite_visual/__init__.py":
                data = shim.encode()
            target.writestr(item, data)
    return out


def self_test(wheel: Path) -> int:
    """Prove every assertion above can go red on a deliberately broken wheel
    (R1). Uses the real wheel as the clean case, so a self-test that passes
    against nothing is not possible."""
    failures: list[str] = []
    observations = 0

    def observe(label: str, got: int, want: int) -> None:
        nonlocal observations
        observations += 1
        if got != want:
            failures.append(f"{label}: expected exit {want}, got {got}")

    assets = expected_assets()
    observe("a real wheel passes inventory", 0 if not check_inventory(wheel, assets) else 1, 0)
    observe("no assets in index.html is not a pass", 1 if check_inventory(wheel, []) else 0, 1)

    with tempfile.TemporaryDirectory() as tmp:
        tmpdir = Path(tmp)

        broken = _doctor(wheel, tmpdir / "no-ext.whl", drop=("kglite_visual/_native",))
        observe("a wheel with no extension fails", 1 if check_inventory(broken, assets) else 0, 1)

        broken = _doctor(wheel, tmpdir / "no-shim.whl", drop=("kglite_visual/_notebook.py",))
        observe("a wheel missing a shim module fails", 1 if check_inventory(broken, assets) else 0, 1)

        broken = _doctor(wheel, tmpdir / "no-entry.whl", drop=("kglite_visual-",))
        observe("a wheel with no dist-info fails", 1 if check_inventory(broken, assets) else 0, 1)

        # The one this check exists for: a wheel built against an empty
        # frontend/dist. Simulated by an extension carrying none of the bundle.
        broken = _doctor(wheel, tmpdir / "no-assets.whl", stub_so=True)
        problems = check_inventory(broken, assets)
        observe("a wheel without the embedded bundle fails", 1 if problems else 0, 1)
        if problems and not any("not embedded" in p for p in problems):
            failures.append(
                f"the no-bundle wheel failed for the wrong reason: {problems}"
            )

        # And the consumer half, seen failing: a shim that imports but has no
        # show(). The probe must go red rather than reporting success.
        broken = _doctor(
            wheel,
            tmpdir / "no-show.whl",
            shim="__version__ = '0.0.0'\n",
        )
        consumer_problems, _ = check_consumer(broken)
        observe("the consumer probe fails on a shim without show()",
                1 if consumer_problems else 0, 1)

    if observations == 0:
        print("check-wheel: FAIL — the self-test asserted nothing", file=sys.stderr)
        return 2
    if failures:
        print("check-wheel --self-test: FAIL", file=sys.stderr)
        for failure in failures:
            print(f"  {failure}", file=sys.stderr)
        return 1
    print(f"check-wheel --self-test: OK — {observations} observations")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--wheel", type=Path, help="default: newest in target/wheels")
    parser.add_argument(
        "--inventory-only",
        action="store_true",
        help="skip the fresh-venv consumer probe",
    )
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()

    wheel = args.wheel or newest_wheel(WHEEL_DIR)
    if wheel is None:
        print(
            f"check-wheel: could not run — no wheel in {WHEEL_DIR}.\n"
            "  Build one with `make wheel`.",
            file=sys.stderr,
        )
        return 2
    if not wheel.is_file():
        print(f"check-wheel: could not run — no such wheel: {wheel}", file=sys.stderr)
        return 2

    if args.self_test:
        return self_test(wheel)
    return check(wheel, skip_consumer=args.inventory_only)


if __name__ == "__main__":
    sys.exit(main())
