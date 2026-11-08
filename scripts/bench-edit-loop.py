#!/usr/bin/env python3
"""Edit-loop timings for plain Cargo, one or more Kache builds, and mbx.

Two checkouts of one revision stand in for two worktrees. Every arm keeps
Cargo's normal incremental policy (CARGO_INCREMENTAL unset), each sample owns
a fresh store and fresh target directories, and arm order rotates between
samples. Raw samples go to --output as JSON; a median table goes to stdout.

    scripts/bench-edit-loop.py --checkout-a A --checkout-b B \
        --kache passthrough=/path/to/kache --kache adaptive=/path/to/kache \
        --mbx /path/to/mbx --samples 4 --output tmp/bench/edit-loop.json
"""

import argparse
import json
import os
import resource
import shutil
import statistics
import subprocess
import sys
import time
from pathlib import Path

EDIT_MARKER = "\npub const KACHE_BENCH_EDIT: u32 = "
WRAPPER_ENV = (
    "KACHE_",
    "MBX_",
    "RUSTC_WRAPPER",
    "CARGO_TARGET_DIR",
    "CARGO_BUILD",
    "CARGO_INCREMENTAL",
    "HOST_CC",
    "HOST_CXX",
    "CC_KNOWN_WRAPPER_CUSTOM",
    "SCCACHE",
)


def arm_env(arm, binary, cache, target, scratch):
    env = {k: v for k, v in os.environ.items() if not k.startswith(WRAPPER_ENV)}
    env["CARGO_TARGET_DIR"] = str(target)
    env["CC_KNOWN_WRAPPER_CUSTOM"] = ""
    env["HOST_CC"] = "clang"
    env["HOST_CXX"] = "clang++"
    # Empty values override a global `build.rustc-wrapper` and `[env]` table.
    env["RUSTC_WRAPPER"] = ""
    if arm.startswith("kache-"):
        env["RUSTC_WRAPPER"] = str(binary)
        env["HOST_CC"] = f"{binary} clang"
        env["HOST_CXX"] = f"{binary} clang++"
        env["KACHE_CACHE_DIR"] = str(cache)
        env["KACHE_CONFIG"] = str(scratch / "empty-config.toml")
    elif arm == "mbx":
        # mbx defers to any configured wrapper, including a global one.
        del env["RUSTC_WRAPPER"]
        env["CARGO_BUILD_RUSTC_WRAPPER"] = ""
        env["MBX_CACHE_DIR"] = str(cache)
        env["MBX_SUMMARY"] = "off"
    return env


def children_cpu():
    usage = resource.getrusage(resource.RUSAGE_CHILDREN)
    return usage.ru_utime + usage.ru_stime


class Arm:
    def __init__(self, name, binary, args, scratch):
        self.name = name
        self.binary = binary
        self.args = args
        self.scratch = scratch

    def build(self, checkout, cache, target):
        cargo = ["build", "--offline", "-q", "-p", self.args.package]
        command = [str(self.binary), *cargo] if self.name == "mbx" else ["cargo", *cargo]
        env = arm_env(self.name, self.binary, cache, target, self.scratch)
        cpu_before = children_cpu()
        started = time.monotonic()
        done = subprocess.run(command, cwd=checkout, env=env, capture_output=True, text=True)
        wall = time.monotonic() - started
        if done.returncode != 0:
            sys.exit(f"{self.name} build failed in {checkout}:\n{done.stderr}")
        if self.name == "mbx" and "deferring to it" in done.stderr:
            sys.exit(f"mbx deferred to another wrapper:\n{done.stderr}")
        return {"wall_s": round(wall, 3), "cpu_s": round(children_cpu() - cpu_before, 3)}

    def last_event(self, cache):
        events = cache / "events.jsonl"
        if not self.name.startswith("kache-") or not events.exists():
            return None
        crate = self.args.package.replace("-", "_")
        for line in reversed(events.read_text().splitlines()):
            try:
                event = json.loads(line)
            except ValueError:
                continue
            if event.get("crate_name") == crate:
                return {"result": event.get("result"), "reason": event.get("passthrough_reason")}
        return None

    def stop(self, cache):
        if self.name.startswith("kache-"):
            env = arm_env(self.name, self.binary, cache, self.scratch / "unused", self.scratch)
            subprocess.run([str(self.binary), "daemon", "stop"], env=env, capture_output=True)


def edit(checkout, path, n):
    source = checkout / path
    text = source.read_text()
    if EDIT_MARKER in text:
        text = text[: text.index(EDIT_MARKER)]
    source.write_text(f"{text}{EDIT_MARKER}{n};\n")


def restore(checkout, path):
    subprocess.run(["git", "checkout", "--", path], cwd=checkout, check=True)


def allocated(path):
    done = subprocess.run(["du", "-sk", str(path)], capture_output=True, text=True)
    return int(done.stdout.split()[0]) * 1024 if done.returncode == 0 and done.stdout else None


def remove_tree(path):
    if not path.exists():
        return
    subprocess.run(["chmod", "-R", "u+w", str(path)], check=False)
    shutil.rmtree(path, ignore_errors=True)


def sample(arm, index, args, scratch):
    root = scratch / f"{arm.name}-{index}"
    remove_tree(root)
    root.mkdir(parents=True)
    cache = root / "cache"
    target_a, target_a2, target_b = root / "target-a", root / "target-a2", root / "target-b"
    a, b = args.checkout_a, args.checkout_b
    restore(a, args.edit)
    restore(b, args.edit)
    phases = {}

    def record(name, result):
        result["event"] = arm.last_event(cache)
        phases[name] = result

    record("cold", arm.build(a, cache, target_a))
    record("warm_store_empty_target_same_checkout", arm.build(a, cache, target_a2))
    record("warm_store_empty_target_other_checkout", arm.build(b, cache, target_b))
    record("noop_intact_target", arm.build(a, cache, target_a))
    for n in range(1, args.edits + 1):
        edit(a, args.edit, n)
        record(f"edit_{n}", arm.build(a, cache, target_a))
    restore(a, args.edit)
    record("revert_to_cached_source", arm.build(a, cache, target_a))
    phases["allocated_bytes"] = {
        "store": allocated(cache) if cache.exists() else 0,
        "target": allocated(target_a),
    }
    arm.stop(cache)
    remove_tree(root)
    return phases


def version(command):
    done = subprocess.run(command, capture_output=True, text=True)
    return done.stdout.strip() or done.stderr.strip()


def summarize(results, arms):
    rows = {}
    for record in results["samples"]:
        for phase, value in record["phases"].items():
            if "wall_s" in value:
                rows.setdefault(phase, {}).setdefault(record["arm"], []).append(value["wall_s"])
    names = [arm.name for arm in arms]
    print(f"{'wall seconds, median [min-max]':42}" + "".join(f"{name:>22}" for name in names))
    for phase, by_arm in rows.items():
        cells = []
        for name in names:
            walls = by_arm.get(name, [])
            cells.append(
                f"{statistics.median(walls):.2f} [{min(walls):.2f}-{max(walls):.2f}]" if walls else "-"
            )
        print(f"{phase:42}" + "".join(f"{cell:>22}" for cell in cells))


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--checkout-a", type=Path, required=True)
    parser.add_argument("--checkout-b", type=Path, required=True)
    parser.add_argument("--kache", action="append", default=[], metavar="NAME=PATH")
    parser.add_argument("--mbx", type=Path)
    parser.add_argument("--package", default="kache-store")
    parser.add_argument("--edit", default="crates/kache-store/src/lib.rs")
    parser.add_argument("--edits", type=int, default=5)
    parser.add_argument("--samples", type=int, default=4)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    args.checkout_a = args.checkout_a.resolve()
    args.checkout_b = args.checkout_b.resolve()
    args.output = args.output.resolve()

    scratch = args.output.parent / "edit-loop-scratch"
    scratch.mkdir(parents=True, exist_ok=True)
    (scratch / "empty-config.toml").write_text("")
    arms = [Arm("plain", None, args, scratch)]
    for spec in args.kache:
        name, _, path = spec.partition("=")
        arms.append(Arm(f"kache-{name}", Path(path).resolve(), args, scratch))
    if args.mbx:
        arms.append(Arm("mbx", args.mbx.resolve(), args, scratch))

    results = {
        "versions": {
            "cargo": version(["cargo", "--version"]),
            "rustc": version(["rustc", "--version"]),
            "checkout": version(["git", "-C", str(args.checkout_a), "rev-parse", "HEAD"]),
            **{arm.name: version([str(arm.binary), "--version"]) for arm in arms if arm.binary},
        },
        "package": args.package,
        "edit": args.edit,
        "cargo_incremental": "unset",
        "samples": [],
    }
    for index in range(args.samples):
        shift = index % len(arms)
        for arm in arms[shift:] + arms[:shift]:
            results["samples"].append({"arm": arm.name, "index": index, "phases": sample(arm, index, args, scratch)})
            args.output.write_text(json.dumps(results, indent=2))
            print(f"sample {index} {arm.name} done", file=sys.stderr, flush=True)
    restore(args.checkout_a, args.edit)
    summarize(results, arms)


if __name__ == "__main__":
    main()
