#!/usr/bin/env python3
"""Copy embedded inputs into the generated output and reference OUT_DIR."""
import ast
import re
import shutil
import sys
from pathlib import Path

source, output, package = Path(sys.argv[1]), Path(sys.argv[2]), Path(sys.argv[3])
shutil.copytree(source, output, dirs_exist_ok=True)
resources = output / "resources"
resources.mkdir()


def embedded(match):
    original = ast.literal_eval(match[1])
    relative = original.rsplit("/_main/", 1)[-1]
    path = Path(relative)
    target = resources / f"{len(list(resources.iterdir()))}{path.suffix}"
    shutil.copy2(path, target)
    return 'include_bytes!(concat!(env!("OUT_DIR"), "/resources/' + target.name + '"))'


for generated in sorted(output.rglob("*.rs")):
    text = generated.read_text()
    text = re.sub(r'include_bytes\s*!\s*\(\s*("(?:[^"\\]|\\.)*")\s*\)', embedded, text)
    marker = 'include!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/receiver_tests.rs"))'
    if marker in text:
        shutil.copy2(package / "src/receiver_tests.rs", output / "receiver_tests.rs")
        text = text.replace(marker, 'include!(concat!(env!("OUT_DIR"), "/receiver_tests.rs"))')
    generated.chmod(0o644)
    generated.write_text(text)
