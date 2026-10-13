#!/usr/bin/env python3
"""Check artifact handoff and checksum failures without building or publishing."""
import hashlib
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

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
        for name in ("release.sh", "host-release/check.py", "host-release/dmg.py"):
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

    def test_reject_missing_host_worker(self):
        result = self.stage(hosts=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("missing desktop/CLI packages for macos-aarch64", result.stderr)
        self.assertFalse((self.output / "RELEASE.txt").exists())

    def test_reject_corrupted_worker_download(self):
        digest = hashlib.sha256(self.apk.read_bytes()).hexdigest()
        (self.output / "SHA256SUMS-android").write_text(f"{digest}  {self.apk.name}\n")
        self.apk.write_bytes(b"truncated download")
        result = self.stage()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(self.apk.name, result.stderr)
        self.assertFalse((self.output / "RELEASE.txt").exists())


if __name__ == "__main__":
    unittest.main()
