#!/usr/bin/env python3
"""Check generated Rust survives moving to a different action sandbox."""
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

NORMALIZER = Path(__file__).with_name("normalize-generated.py")


class GeneratedResources(unittest.TestCase):
    def test_resources_survive_removing_the_producer_sandbox(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "producer"
            source.mkdir()
            font = root / "app/font.ttf"
            font.parent.mkdir()
            font.write_bytes(b"font data")
            package = root / "crates/android/src"
            package.mkdir(parents=True)
            receiver = 'pub fn receiver() {}\n'
            (package / "receiver_tests.rs").write_text(receiver)
            (source / "generated.rs").write_text(
                'pub static FONT: &[u8] = include_bytes!("/removed/sandbox/execroot/_main/app/font.ttf");\n'
                'include!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/receiver_tests.rs"));\n')
            (source / "generated.rs").chmod(0o444)
            output = root / "consumer"
            output.mkdir()
            subprocess.run([sys.executable, str(NORMALIZER), str(source), str(output), "crates/android"], cwd=root, check=True)
            font.unlink()
            shutil.rmtree(source)
            shutil.rmtree(package.parent)
            generated = (output / "generated.rs").read_text()
            self.assertNotIn("/removed/sandbox", generated)
            self.assertNotIn("CARGO_MANIFEST_DIR", generated)
            self.assertEqual((output / "resources/0.ttf").read_bytes(), b"font data")
            self.assertEqual((output / "receiver_tests.rs").read_text(), receiver)
            subprocess.run(['rustc', '--crate-type=lib', str(output / 'generated.rs'), '-o', str(root / 'generated.rlib')],
                           env=dict(os.environ, OUT_DIR=str(output)), check=True)


if __name__ == "__main__":
    unittest.main()
