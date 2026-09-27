#!/usr/bin/env python3
"""Check integration suites against Cargo's discovered test targets."""

import json
import os
import pathlib
import re
import shutil
import subprocess
import sys
import tempfile
import unittest


def cargo_packages():
    """Use Cargo's own target discovery, rather than guessing from main.rs."""
    command = ["cargo", "metadata", "--locked", "--offline", "--no-deps", "--format-version", "1"]
    try:
        result = subprocess.run(command, capture_output=True, text=True, check=False)
    except FileNotFoundError:
        print("test-modules: cargo is required to discover integration test targets", file=sys.stderr)
        return None
    if result.returncode:
        print("test-modules: cargo metadata failed:\n" + result.stderr, file=sys.stderr)
        return None
    return json.loads(result.stdout)["packages"]


def declared(owner, name):
    if not owner.is_file():
        return False
    pattern = rf"(?m)^\s*(?:#\[cfg\([^]]+\)\]\s*)?mod\s+{re.escape(name)}\s*;"
    return re.search(pattern, owner.read_text()) is not None


def check_integration_suites():
    packages = cargo_packages()
    if packages is None:
        return 1
    root = pathlib.Path.cwd()
    targets = []
    for package in packages:
        for target in package["targets"]:
            if "test" in target["kind"] and target.get("test", True):
                targets.append(pathlib.Path(target["src_path"]))

    # A #[path] import can share a support module across test binaries (and
    # even crates). Such a directory is not itself a Cargo integration suite.
    imported_support = set()
    path_module = r'#\[\s*path\s*=\s*"([^"]+)"\s*\]\s*mod\s+\w+\s*;'
    for target in targets:
        for match in re.finditer(path_module, target.read_text()):
            imported_support.add(pathlib.Path(os.path.normpath(target.parent / match.group(1))))

    failed = False
    for package in packages:
        package_dir = pathlib.Path(package["manifest_path"]).parent
        tests_dir = package_dir / "tests"
        if not tests_dir.is_dir():
            continue
        for suite in sorted(tests_dir.iterdir()):
            if not suite.is_dir():
                continue
            files = sorted(suite.rglob("*.rs"))
            if not files:
                continue
            entries = [target for target in targets if target.is_relative_to(suite)]
            if not entries:
                entries = [file for file in files if file in imported_support]
                shared_by_module = any(
                    target.parent == tests_dir and declared(target, suite.name)
                    for target in targets
                )
                if shared_by_module and (suite / "mod.rs").is_file():
                    entries.append(suite / "mod.rs")
            if not entries:
                missed = suite / "main.rs" if (suite / "main.rs").is_file() else files[0]
                print(
                    f"test-modules: {missed.relative_to(root)} is in suite "
                    f"{suite.relative_to(root)} with no Cargo integration test target "
                    "(add main.rs or register a [[test]] target)",
                    file=sys.stderr,
                )
                failed = True
                continue

            for file in files:
                if file in entries:
                    continue
                if file.name == "mod.rs":
                    name = file.parent.name
                    owner_dir = file.parent.parent
                else:
                    name = file.stem
                    owner_dir = file.parent
                if owner_dir == suite:
                    owners = entries
                else:
                    owners = [owner_dir / "mod.rs", owner_dir / "main.rs", pathlib.Path(str(owner_dir) + ".rs")]
                if not any(declared(owner, name) for owner in owners):
                    print(
                        f"test-modules: {file.relative_to(root)} is not declared "
                        f"by its suite {suite.relative_to(root)} "
                        f"(add `mod {name};` to the module that owns {owner_dir.relative_to(root)})",
                        file=sys.stderr,
                    )
                    failed = True
    return int(failed)


class GuardFixtures(unittest.TestCase):
    def setUp(self):
        scratch = pathlib.Path(__file__).absolute().parent.parent / ".orbit" / "tmp"
        scratch.mkdir(parents=True, exist_ok=True)
        self.temp = tempfile.TemporaryDirectory(dir=scratch)
        self.addCleanup(self.temp.cleanup)
        self.root = pathlib.Path(self.temp.name)
        (self.root / "scripts").mkdir()
        shutil.copy2(pathlib.Path(__file__).absolute().parent.parent / "rust-toolchain.toml", self.root / "rust-toolchain.toml")
        for name in ("check-test-modules.sh", "check-integration-test-modules.py"):
            shutil.copy2(pathlib.Path(__file__).parent / name, self.root / "scripts" / name)
        (self.root / "Cargo.toml").write_text(
            '[workspace]\nmembers = ["crates/demo"]\nresolver = "2"\n'
        )
        self.put("crates/demo/Cargo.toml", '[package]\nname = "demo"\nversion = "0.1.0"\nedition = "2021"\n')
        self.put("crates/demo/src/lib.rs", "pub fn demo() {}\n")
        (self.root / "apps/desktop/src-tauri/src").mkdir(parents=True)
        (self.root / "tmp").mkdir()
        self.env = {key: value for key, value in os.environ.items() if not key.startswith(("GIT_", "ORBIT_"))}
        for key in ("NEBULA_ROOT", "OBSERVATORY_ROOT", "EDITOR", "VISUAL", "CARGO_TARGET_DIR"):
            self.env.pop(key, None)
        rustup_home = os.environ.get("RUSTUP_HOME", str(pathlib.Path.home() / ".rustup"))
        self.env.update(
            HOME=str(self.root),
            CARGO_HOME=str(self.root / ".cargo"),
            RUSTUP_HOME=rustup_home,
            TMPDIR=str(self.root / "tmp"),
        )
        subprocess.run(
            ["cargo", "generate-lockfile", "--offline"],
            cwd=self.root,
            env=self.env,
            check=True,
            capture_output=True,
        )

    def put(self, name, contents):
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(contents)

    def run_guard(self):
        return subprocess.run(
            ["bash", str(self.root / "scripts/check-test-modules.sh")],
            cwd=self.root,
            env=self.env,
            capture_output=True,
            text=True,
            check=False,
        )

    def test_missing_main_is_named(self):
        self.put("crates/demo/tests/lost/case.rs", '#[test] fn never_runs() { panic!("must run"); }\n')
        result = self.run_guard()
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("tests/lost/case.rs", result.stderr)

    def test_autotests_false_requires_registered_target(self):
        self.put("crates/demo/Cargo.toml", '[package]\nname = "demo"\nversion = "0.1.0"\nedition = "2021"\nautotests = false\n')
        self.put("crates/demo/tests/lost/main.rs", "mod case;\n")
        self.put("crates/demo/tests/lost/case.rs", "#[test] fn never_runs() {}\n")
        result = self.run_guard()
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("tests/lost/main.rs", result.stderr)

    def test_unlinked_file_in_registered_suite_is_named(self):
        self.put("crates/demo/tests/lost/main.rs", "fn main() {}\n")
        self.put("crates/demo/tests/lost/case.rs", "#[test] fn never_runs() {}\n")
        result = self.run_guard()
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("tests/lost/case.rs", result.stderr)

    def test_disabled_explicit_target_is_not_registered(self):
        self.put("crates/demo/Cargo.toml", '[package]\nname = "demo"\nversion = "0.1.0"\nedition = "2021"\nautotests = false\n\n[[test]]\nname = "lost"\npath = "tests/lost/main.rs"\ntest = false\n')
        self.put("crates/demo/tests/lost/main.rs", "mod case;\n")
        self.put("crates/demo/tests/lost/case.rs", "#[test] fn never_runs() {}\n")
        result = self.run_guard()
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("tests/lost/main.rs", result.stderr)

    def test_explicit_target_registers_suite(self):
        self.put("crates/demo/Cargo.toml", '[package]\nname = "demo"\nversion = "0.1.0"\nedition = "2021"\nautotests = false\n\n[[test]]\nname = "active"\npath = "tests/active/main.rs"\n')
        self.put("crates/demo/tests/active/main.rs", "mod case;\n")
        self.put("crates/demo/tests/active/case.rs", "#[test] fn runs() {}\n")
        result = self.run_guard()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_explicit_alternate_entry_registers_suite(self):
        self.put("crates/demo/Cargo.toml", '[package]\nname = "demo"\nversion = "0.1.0"\nedition = "2021"\nautotests = false\n\n[[test]]\nname = "active"\npath = "tests/active/entry.rs"\n')
        self.put("crates/demo/tests/active/entry.rs", "mod case;\n")
        self.put("crates/demo/tests/active/case.rs", "#[test] fn runs() {}\n")
        result = self.run_guard()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_shared_support_is_not_a_suite(self):
        self.put("crates/demo/tests/active/main.rs", '#[path = "../support/mod.rs"]\nmod support;\nmod case;\n')
        self.put("crates/demo/tests/active/case.rs", "#[test] fn runs() {}\n")
        self.put("crates/demo/tests/support/mod.rs", "#[test] fn shared_test() {}\n")
        result = self.run_guard()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_shared_module_declared_by_standalone_target(self):
        self.put("crates/demo/tests/commands.rs", "mod lock_holder;\n#[test] fn runs() {}\n")
        self.put("crates/demo/tests/lock_holder/mod.rs", "pub fn hold() {}\n")
        result = self.run_guard()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)


if __name__ == "__main__":
    if sys.argv[1:] == ["--self-test"]:
        unittest.main(argv=[sys.argv[0]])
    elif sys.argv[1:] == ["--check"]:
        sys.exit(check_integration_suites())
    else:
        sys.exit("usage: check-integration-test-modules.py --check|--self-test")
