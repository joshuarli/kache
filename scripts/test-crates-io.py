#!/usr/bin/env python3
"""Exercise publishable-crate discovery and crates.io helpers."""

import contextlib
import importlib.util
import io
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock


def load_mod():
    path = Path(__file__).with_name("crates-io.py")
    spec = importlib.util.spec_from_file_location("crates_io", path)
    if spec is None or spec.loader is None:
        raise ImportError(f"cannot load {path}")
    module = importlib.util.module_from_spec(spec)
    sys.modules["crates_io"] = module
    spec.loader.exec_module(module)
    return module


class CratesIoTests(unittest.TestCase):
    def setUp(self):
        self.mod = load_mod()
        self.scratch = tempfile.TemporaryDirectory()
        self.addCleanup(self.scratch.cleanup)
        self.root = Path(self.scratch.name)
        self.write(
            "Cargo.toml",
            '[package]\nname = "kache"\nversion = "0.17.0"\n'
            "[workspace]\nmembers = [\"crates/*\"]\n"
            '[dependencies]\n'
            'kache-format = { path = "crates/kache-format", version = "0.17.0" }\n'
            'kache-store = { path = "crates/kache-store", version = "0.17.0" }\n',
        )
        self.write(
            "crates/kache-format/Cargo.toml",
            '[package]\nname = "kache-format"\nversion = "0.17.0"\n',
        )
        self.write(
            "crates/kache-store/Cargo.toml",
            '[package]\nname = "kache-store"\nversion = "0.17.0"\n'
            '[dependencies]\n'
            'kache-format = { path = "../kache-format", version = "0.17.0" }\n'
            'kache-fs = { path = "../kache-fs", version = "0.17.0" }\n',
        )
        self.write(
            "crates/kache-fs/Cargo.toml",
            '[package]\nname = "kache-fs"\nversion = "0.17.0"\n',
        )
        self.write(
            "crates/kache-core/Cargo.toml",
            '[package]\nname = "kache-core"\nversion = "0.17.0"\n',
        )
        self.write(
            "crates/kache-e2e/Cargo.toml",
            '[package]\nname = "kache-e2e"\nversion = "0.17.0"\npublish = false\n',
        )

    def write(self, name, text):
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)

    def names(self):
        return [crate.name for crate in self.mod.load_publishable(self.root)]

    def test_unpublished_members_are_skipped(self):
        self.assertNotIn("kache-e2e", self.names())

    def test_dependency_order_puts_leaves_first(self):
        names = self.names()
        self.assertEqual(
            names,
            ["kache-core", "kache-format", "kache-fs", "kache-store", "kache"],
        )
        self.assertLess(names.index("kache-format"), names.index("kache-store"))
        self.assertLess(names.index("kache-fs"), names.index("kache-store"))
        self.assertLess(names.index("kache-store"), names.index("kache"))

    def test_new_publishable_crate_is_listed_without_a_script_edit(self):
        self.write(
            "crates/kache-new/Cargo.toml",
            '[package]\nname = "kache-new"\nversion = "0.17.0"\n',
        )
        names = self.names()
        self.assertIn("kache-new", names)
        self.assertLess(names.index("kache-new"), names.index("kache"))

    def test_empty_publish_list_is_unpublished(self):
        self.write(
            "crates/kache-internal/Cargo.toml",
            '[package]\nname = "kache-internal"\nversion = "0.17.0"\npublish = []\n',
        )
        self.assertNotIn("kache-internal", self.names())

    def test_cargo_p_args_cover_every_publishable_crate(self):
        crates = self.mod.load_publishable(self.root)
        args = self.mod.cargo_p_args(crates)
        self.assertEqual(
            args,
            [
                "-p",
                "kache-core",
                "-p",
                "kache-format",
                "-p",
                "kache-fs",
                "-p",
                "kache-store",
                "-p",
                "kache",
            ],
        )

    def test_cli_list_matches_discovery(self):
        result = subprocess.run(
            [
                "python3",
                str(Path(__file__).with_name("crates-io.py")),
                "--root",
                str(self.root),
                "list",
            ],
            text=True,
            capture_output=True,
            check=False,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.split(), self.names())

    def test_sparse_index_path(self):
        self.assertEqual(self.mod.sparse_index_path("a"), "1/a")
        self.assertEqual(self.mod.sparse_index_path("ab"), "2/ab")
        self.assertEqual(self.mod.sparse_index_path("abc"), "3/a/abc")
        self.assertEqual(self.mod.sparse_index_path("kache"), "ka/ch/kache")
        self.assertEqual(self.mod.sparse_index_path("kache-core"), "ka/ch/kache-core")

    def test_desired_config_matches_publish_workflow_without_environment(self):
        config = {
            "repository_owner": "kunobi-ninja",
            "repository_name": "kache",
            "workflow_filename": "publish-crates.yaml",
            "environment": None,
        }
        self.assertTrue(self.mod.is_desired_config(config))
        self.assertTrue(self.mod.is_desired_config({**config, "environment": ""}))
        self.assertFalse(self.mod.is_desired_config({**config, "environment": "release"}))
        self.assertFalse(
            self.mod.is_desired_config({**config, "workflow_filename": "publish-crates.yml"})
        )

    def test_new_crate_hint_names_the_make_recipe(self):
        hint = self.mod.new_crate_hint("kache-format")
        self.assertIn("make crates-bootstrap", hint)
        self.assertIn("kache-format", hint)
        self.assertIn("cannot create", hint.lower())

    def test_cycle_is_rejected(self):
        self.write(
            "crates/kache-format/Cargo.toml",
            '[package]\nname = "kache-format"\nversion = "0.17.0"\n'
            '[dependencies]\n'
            'kache-store = { path = "../kache-store", version = "0.17.0" }\n',
        )
        with self.assertRaises(SystemExit) as raised:
            self.mod.load_publishable(self.root)
        self.assertIn("cycle", str(raised.exception))


class FakeClient:
    def __init__(self, published=(), known=(), visible=True):
        self.published = set(published)
        self.known = set(known)
        self.visible = visible
        self.index_checks = []

    def version_exists(self, name, version):
        return (name, version) in self.published

    def crate(self, name):
        return {"name": name} if name in self.known else None

    def index_has_version(self, name, version):
        self.index_checks.append((name, version))
        return self.visible


class PublishGateTests(unittest.TestCase):
    def setUp(self):
        self.mod = load_mod()
        root = Path("/unused")
        self.crates = [
            self.mod.Crate("kache-format", "1.2.3", root),
            self.mod.Crate("kache-store", "1.2.3", root),
        ]
        self.cargo_calls = []
        patcher = mock.patch.object(
            self.mod, "run_cargo", lambda _root, args: self.cargo_calls.append(args)
        )
        patcher.start()
        self.addCleanup(patcher.stop)

    def publish(self, client, *, version="1.2.3", dry_run=False):
        with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(
            io.StringIO()
        ):
            return self.mod.cmd_publish(
                self.crates,
                client,
                Path("/unused"),
                version=version,
                dry_run=dry_run,
                sleep=lambda _seconds: None,
            )

    def test_a_version_mismatch_publishes_nothing(self):
        client = FakeClient(known={"kache-format", "kache-store"})
        with self.assertRaises(SystemExit) as raised:
            self.publish(client, version="1.2.4")
        self.assertIn("not all 1.2.4", str(raised.exception))
        self.assertEqual(self.cargo_calls, [])
        self.assertEqual(client.index_checks, [])

    def test_publishing_goes_in_order_and_waits_for_each_index_entry(self):
        client = FakeClient(known={"kache-format", "kache-store"})
        self.assertEqual(self.publish(client), 0)
        self.assertEqual(
            self.cargo_calls,
            [
                ["publish", "-p", "kache-format", "--locked"],
                ["publish", "-p", "kache-store", "--locked"],
            ],
        )
        self.assertEqual(
            client.index_checks,
            [("kache-format", "1.2.3"), ("kache-store", "1.2.3")],
        )

    def test_an_already_published_version_is_not_published_again(self):
        client = FakeClient(
            published={("kache-format", "1.2.3")}, known={"kache-store"}
        )
        self.assertEqual(self.publish(client), 0)
        self.assertEqual(self.cargo_calls, [["publish", "-p", "kache-store", "--locked"]])

    def test_an_unregistered_crate_stops_the_run_before_any_later_publish(self):
        client = FakeClient(known={"kache-store"})
        self.assertEqual(self.publish(client), 1)
        self.assertEqual(self.cargo_calls, [])

    def test_a_dry_run_publishes_and_waits_for_nothing(self):
        client = FakeClient(known={"kache-format", "kache-store"})
        self.assertEqual(self.publish(client, dry_run=True), 0)
        self.assertEqual(self.cargo_calls, [])
        self.assertEqual(client.index_checks, [])

    def test_an_entry_that_never_reaches_the_index_fails_the_run(self):
        client = FakeClient(known={"kache-format", "kache-store"}, visible=False)
        with self.assertRaises(SystemExit) as raised:
            self.publish(client)
        self.assertIn("did not become visible", str(raised.exception))
        self.assertEqual(self.cargo_calls, [["publish", "-p", "kache-format", "--locked"]])


if __name__ == "__main__":
    unittest.main()
