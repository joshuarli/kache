#!/usr/bin/env python3
"""Exercise edit-loop environment construction, edit generators, and reporting."""

import io
import os
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

sys_path_parent = Path(__file__).resolve().parent.parent.parent
import sys
if str(sys_path_parent) not in sys.path:
    sys.path.insert(0, str(sys_path_parent))

import importlib.util
spec = importlib.util.spec_from_file_location(
    "bench_edit_loop",
    sys_path_parent / "bench-edit-loop.py",
)
bench_edit_loop = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bench_edit_loop)


class TestEditLoop(unittest.TestCase):
    def test_arm_env_kache(self):
        scratch = Path("/tmp/scratch")
        target = Path("/tmp/target")
        cache = Path("/tmp/cache")
        binary = Path("/bin/kache")
        env = bench_edit_loop.arm_env("kache-adaptive", binary, cache, target, scratch)
        self.assertEqual(env["RUSTC_WRAPPER"], str(binary))
        self.assertEqual(env["HOST_CC"], f"{binary} clang")
        self.assertEqual(env["HOST_CXX"], f"{binary} clang++")
        self.assertEqual(env["KACHE_CACHE_DIR"], str(cache))
        self.assertEqual(env["KACHE_CONFIG"], str(scratch / "empty-config.toml"))
        self.assertEqual(env["CARGO_TARGET_DIR"], str(target))

    def test_arm_env_mbx(self):
        scratch = Path("/tmp/scratch")
        target = Path("/tmp/target")
        cache = Path("/tmp/cache")
        binary = Path("/bin/mbx")
        with patch.dict(os.environ, {"RUSTC_WRAPPER": "/global/wrapper"}):
            env = bench_edit_loop.arm_env("mbx", binary, cache, target, scratch)
            self.assertNotIn("RUSTC_WRAPPER", env)
            self.assertEqual(env["CARGO_BUILD_RUSTC_WRAPPER"], "")
            self.assertEqual(env["MBX_CACHE_DIR"], str(cache))
            self.assertEqual(env["MBX_SUMMARY"], "off")

    def test_edit_content_sequence(self):
        names = []
        for i in range(1, len(bench_edit_loop.REALISTIC_EDITS) + 1):
            kind, snippet = bench_edit_loop.edit_content(i)
            names.append(kind)
            self.assertIn("bench_validate_key", snippet)
        self.assertEqual(
            names,
            [
                "add_validator_helper",
                "tweak_validator_body",
                "add_batch_consumer",
                "refactor_consumer_dedup",
                "doc_and_error_handling",
            ],
        )

        extra_kind, extra_snippet = bench_edit_loop.edit_content(6)
        self.assertEqual(extra_kind, "additive_helper_6")
        self.assertIn("pub fn bench_extra_helper_6", extra_snippet)

    def test_edit_replaces_marker_cleanly(self):
        with tempfile.TemporaryDirectory() as td:
            checkout = Path(td)
            source_file = checkout / "lib.rs"
            source_file.write_text("pub trait ArtifactPolicy {}\n")

            kind1 = bench_edit_loop.edit(checkout, Path("lib.rs"), 1)
            self.assertEqual(kind1, "add_validator_helper")
            content1 = source_file.read_text()
            self.assertTrue(content1.startswith("pub trait ArtifactPolicy {}\n"))
            self.assertIn(bench_edit_loop.EDIT_MARKER, content1)
            self.assertIn("bench_validate_key", content1)

            kind2 = bench_edit_loop.edit(checkout, Path("lib.rs"), 2)
            self.assertEqual(kind2, "tweak_validator_body")
            content2 = source_file.read_text()
            self.assertEqual(content2.count(bench_edit_loop.EDIT_MARKER), 1)

    def test_edit_replaces_legacy_marker(self):
        with tempfile.TemporaryDirectory() as td:
            checkout = Path(td)
            source_file = checkout / "lib.rs"
            source_file.write_text("pub trait ArtifactPolicy {}\npub const KACHE_BENCH_EDIT: u32 = 1;\n")

            kind = bench_edit_loop.edit(checkout, Path("lib.rs"), 1)
            self.assertEqual(kind, "add_validator_helper")
            content = source_file.read_text()
            self.assertNotIn("pub const KACHE_BENCH_EDIT: u32 = 1;", content)
            self.assertIn(bench_edit_loop.EDIT_MARKER, content)

    def test_children_resources(self):
        cpu_s, max_rss_bytes = bench_edit_loop.children_resources()
        self.assertIsInstance(cpu_s, float)
        self.assertIsInstance(max_rss_bytes, int)
        self.assertGreaterEqual(cpu_s, 0.0)
        self.assertGreaterEqual(max_rss_bytes, 0)

    def test_summarize(self):
        class DummyArm:
            def __init__(self, name):
                self.name = name

        arms = [DummyArm("plain"), DummyArm("kache-adaptive")]
        results = {
            "samples": [
                {
                    "arm": "plain",
                    "phases": {
                        "cold": {"wall_s": 5.0},
                        "edit_1": {"wall_s": 1.0},
                    },
                },
                {
                    "arm": "kache-adaptive",
                    "phases": {
                        "cold": {"wall_s": 6.0},
                        "edit_1": {"wall_s": 0.8},
                    },
                },
            ]
        }
        stdout = io.StringIO()
        with patch("sys.stdout", stdout):
            bench_edit_loop.summarize(results, arms)
        out = stdout.getvalue()
        self.assertIn("cold", out)
        self.assertIn("edit_1", out)
        self.assertIn("5.00", out)
        self.assertIn("0.80", out)


if __name__ == "__main__":
    unittest.main()
