#!/usr/bin/env python3
"""CCP-07: bounded Autotools/libtool launcher check with default path mapping."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def run(argv, cwd, env, log):
    with log.open("a") as output:
        subprocess.run(argv, cwd=cwd, env=env, stdout=output,
                       stderr=subprocess.STDOUT, check=True)


def snapshot(build):
    paths = sorted(p for p in build.rglob("*") if p.is_file()
                   and (p.suffix in {".o", ".lo", ".Po", ".Plo"}))
    require(len([p for p in paths if p.suffix == ".o"]) == 3,
            "expected main.o and both PIC/non-PIC answer.o objects")
    require((build / "answer.lo").is_file(), "missing libtool .lo descriptor")
    require((build / ".libs/answer.o").is_file() and (build / "answer.o").is_file(),
            "missing PIC/non-PIC pair")
    for name in ["answer.Plo", "main.Po"]:
        depfile = build / ".deps" / name
        require(depfile.is_file() and "answer.h" in depfile.read_text(),
                f"missing or incomplete depcomp output {depfile}")
    return {str(p.relative_to(build)): hashlib.sha256(p.read_bytes()).hexdigest()
            for p in paths}


def exercise(kache, work, cc, launcher, negative_control, runtime):
    work.mkdir(parents=True)
    source = work / "tree/source"
    shutil.copytree(Path(__file__).resolve().parent.parent / "test-projects/cc-autotools", source)
    env = os.environ.copy()
    env.pop("RUSTC_WRAPPER", None)
    env.update(KACHE_CACHE_DIR=str(work / "cache"),
               KACHE_RUNTIME_DIR=str(work / "cache"),
               KACHE_SOCKET_PATH=str(Path(runtime) / "daemon.sock"),
               KACHE_CONFIG=str(work / "kache.toml"),
               KACHE_CC_PATH_NORMALIZE="1", KACHE_CC_SOURCE_DATE_EPOCH="passthrough",
               KACHE_MIN_STORE_COMPILE_MS="0", LC_ALL="C")
    Path(env["KACHE_CONFIG"]).write_text("")
    log = work / "build.log"
    run(["autoreconf", "-fi"], source, env, log)
    build = source.parent / "build"
    build.mkdir()
    shim = work / "shims"
    shim.mkdir()
    (shim / cc).symlink_to(kache)
    (shim / ".kache-shims").touch()
    wrapped = f"{kache} {cc}" if launcher == "prefix" else cc
    rows_seen = 0

    def stop():
        run([str(kache), "daemon", "stop"], work, env, log)

    def edit(value):
        header = source / "answer.h"
        header.write_text(f"#define ANSWER {value}\nint answer(void);\n")
        time.sleep(2.1)

    def configure(command, wrapped_env):
        configured = wrapped_env.copy()
        configured.update(CC=command, CFLAGS="-O0 -g0")
        run(["../source/configure", "--enable-shared", "--enable-static"],
            build, configured, log)
        require(any(command in line and "conftest.c" in line
                    for line in (build / "config.log").read_text().splitlines()),
                "configure did not run probes through the selected CC")

    def compile_phase(name, expected, command, configure_first=False, hits=False):
        nonlocal rows_seen
        phase_env = env.copy()
        if launcher == "shim" and not name.startswith("raw"):
            phase_env["PATH"] = str(shim) + os.pathsep + env["PATH"]
        if negative_control and name == "warm":
            phase_env["KACHE_DISABLED"] = "1"
        if configure_first:
            configure(command, phase_env)
        if name not in {"raw-edit", "edit", "revert"}:
            run(["make", "clean"], build, phase_env, log)
            for depfile in (build / ".deps").glob("*"):
                depfile.unlink()
        run(["make", "V=1", "all", "check"], build, phase_env, log)
        output = subprocess.check_output([str(build / "check-answer")],
                                         cwd=build, env=phase_env, text=True)
        require(output == f"{expected}\n", f"{name}: wrong linked program output {output!r}")
        files = snapshot(build)
        if name.startswith("raw"):
            return files
        stop()
        rows = [json.loads(line) for line in (work / "cache/events.jsonl").read_text().splitlines()]
        current = [row for row in rows[rows_seen:]
                   if row.get("crate_name") in {"answer.c", "main.c"}]
        rows_seen = len(rows)
        want = "local_hit" if hits else "miss"
        require(len(current) == 3 and all(row["result"] == want for row in current),
                f"{name}: expected 3 {want} events, got {[(r['crate_name'], r['result']) for r in current]}")
        if hits:
            require(all(row["compiler_runs"] == 0 for row in current),
                    f"{name}: warm build generated code")
        (work / f"{name}.json").write_text(json.dumps({"files": files, "events": current}, indent=2))
        print(f"{cc} {launcher} {name}: 3 {want}; objects, .lo and depfiles present", flush=True)
        return files

    # Raw and wrapped builds share paths and CFLAGS; all outputs must match.
    raw = compile_phase("raw", 42, cc, configure_first=True)
    edit(43)
    raw_edit = compile_phase("raw-edit", 43, cc)
    require(raw != raw_edit, "header edit did not change outputs")
    edit(42)
    cold = compile_phase("cold", 42, wrapped, configure_first=True)
    require(cold == raw, "cold objects/descriptors/depfiles differ from raw")
    warm = compile_phase("warm", 42, wrapped, hits=True)
    require(warm == raw, "warm objects/descriptors/depfiles differ from raw")
    edit(43)
    changed = compile_phase("edit", 43, wrapped)
    require(changed == raw_edit, "edited outputs differ from raw")
    edit(42)
    reverted = compile_phase("revert", 42, wrapped, hits=True)
    require(reverted == raw, "reverted outputs differ from raw")
    relocated = work / "relocated"
    source.parent.rename(relocated)
    source = relocated / "source"
    build = relocated / "build"
    shutil.rmtree(build)
    build.mkdir()
    moved = compile_phase("relocated", 42, wrapped, configure_first=True, hits=True)
    require(moved == raw, "relocated outputs differ from raw")
    text = log.read_text()
    require("--mode=compile" in text and all(flag in text for flag in ["-MD", "-MP", "-MF"]),
            "libtool/Automake dependency compile flags were not exercised")
    print(f"PASS: {cc} {launcher}: configure, PIC/non-PIC, depcomp, cold/warm, edit/revert, relocation", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("kache", type=Path)
    parser.add_argument("work", type=Path)
    parser.add_argument("compilers", nargs="*", default=["cc"])
    parser.add_argument("--negative-control", action="store_true",
                        help="disable caching for the warm build; must fail")
    args = parser.parse_args()
    for tool in ["autoreconf", "autoconf", "automake", "libtoolize", "make", *args.compilers]:
        require(shutil.which(tool), f"required tool not found: {tool}")
    root = args.work.resolve()
    root.mkdir(parents=True, exist_ok=True)
    # A fresh run avoids consuming old events or deleting another run's evidence.
    run_root = Path(tempfile.mkdtemp(prefix="run-", dir=root))
    print(f"Evidence: {run_root}", flush=True)
    for cc in args.compilers:
        require(Path(cc).name == cc, "use a compiler name on PATH")
        for launcher in ["prefix", "shim"]:
            with tempfile.TemporaryDirectory(prefix="kache-autotools-") as runtime:
                exercise(args.kache.resolve(), run_root / cc / launcher, cc, launcher,
                         args.negative_control, runtime)


if __name__ == "__main__":
    main()
