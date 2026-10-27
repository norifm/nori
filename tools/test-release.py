#!/usr/bin/env python3
"""Check artifact handoff and checksum failures without building or publishing."""
import hashlib
import os
import runpy
import sys
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parent.parent
VERSION = "1.2.3-beta.4"


class Release(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="nori-release-test-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        (self.root / "tools/host-release").mkdir(parents=True)
        (self.root / "app").mkdir()
        (self.root / "app/build.gradle.kts").write_text(f'versionName = "{VERSION}"\n')
        for name in ("release.sh", "host-release/check.py", "host-release/dmg.py", "host-release/app.py"):
            shutil.copy2(ROOT / "tools" / name, self.root / "tools" / name)
        (self.root / "gradlew").write_text("#!/bin/sh\necho 'unexpected build' >&2\nexit 99\n")
        (self.root / "gradlew").chmod(0o755)
        self.output = self.root / "artifacts"
        self.output.mkdir()
        self.apk = self.output / f"nori-music-{VERSION}.apk"
        self.apk.write_bytes(b"the worker's signed APK")
        self.environment = dict(os.environ, ANDROID_HOME=str(self.root / "sdk"))
        self.environment.pop("NORI_RELEASE_WORKSPACE", None)

    def stage(self, hosts=False):
        return subprocess.run(["bash", str(self.root / "tools/release.sh"), "--build", "--from", str(self.output),
                               "--no-ipod", *([] if hosts else ["--no-hosts"])],
                              env=self.environment, capture_output=True, text=True)

    def test_stage_worker_artifacts_without_building(self):
        result = self.stage()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        digest, name = (self.output / "SHA256SUMS").read_text().strip().split("  ")
        self.assertEqual(name, self.apk.name)
        self.assertEqual(digest, hashlib.sha256(self.apk.read_bytes()).hexdigest())
        self.assertFalse((self.root / "nori-release.jks").exists())
        self.assertFalse((self.root / "keystore.properties").exists())

    def test_missing_host_worker_is_left_out(self):
        result = self.stage(hosts=True)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("missing desktop/CLI packages for: macos-aarch64 macos-x86_64 linux-aarch64 linux-x86_64", result.stderr)
        self.assertTrue((self.output / "RELEASE.txt").exists())

    def test_reject_corrupted_worker_download(self):
        digest = hashlib.sha256(self.apk.read_bytes()).hexdigest()
        (self.output / "SHA256SUMS-android").write_text(f"{digest}  {self.apk.name}\n")
        self.apk.write_bytes(b"truncated download")
        result = self.stage()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(self.apk.name, result.stderr)
        self.assertFalse((self.output / "RELEASE.txt").exists())


class EmulatedSandbox(unittest.TestCase):
    def test_preserve_environment_and_program_arguments(self):
        with tempfile.TemporaryFile() as arguments:
            arguments.write(b"\0".join([
                b"--setenv", b"EMPTY", b"", b"--seccomp", b"42",
                b"--ro-bind", b"/runtime", b"/usr", b"--", b"nori-cli", b"--seccomp",
            ]) + b"\0")
            arguments.seek(0)
            with patch.object(sys, "argv", ["bwrap", "--args", str(arguments.fileno())]), patch("os.execv") as execute:
                runpy.run_path(str(ROOT / "tools/host-release/emulated-bwrap.py"))
            execute.assert_called_once_with("/usr/bin/bwrap", [
                "bwrap", "--setenv", "EMPTY", "", "--ro-bind", "/runtime", "/usr", "--", "nori-cli", "--seccomp",
            ])


class MacCrossBuild(unittest.TestCase):
    def setUp(self):
        self.builder = runpy.run_path(str(ROOT / "tools/host-release/build.py"))

    def test_noninteractive_build_requires_acknowledgment(self):
        with patch("sys.stdin.isatty", return_value=False), self.assertRaises(SystemExit):
            self.builder["confirm_cross_tests"](False)

    def test_confirmation_accepts_only_yes(self):
        for answer in ("n", "", "yes", "y", "Y"):
            with self.subTest(answer=answer), patch("sys.stdin.isatty", return_value=True), patch("builtins.input", return_value=answer):
                if answer.lower() in ("y", "yes"):
                    self.builder["confirm_cross_tests"](False)
                else:
                    with self.assertRaises(SystemExit):
                        self.builder["confirm_cross_tests"](False)

    def test_explicit_acknowledgment_never_prompts(self):
        with patch("builtins.input", side_effect=AssertionError("worker prompted")):
            self.builder["confirm_cross_tests"](True)


if __name__ == "__main__":
    unittest.main()
