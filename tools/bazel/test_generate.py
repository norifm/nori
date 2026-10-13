#!/usr/bin/env python3
"""Check Cargo feature unification and platform dependency selection."""
import runpy
import unittest
from pathlib import Path

GENERATOR = runpy.run_path(str(Path(__file__).with_name("generate.py")))


def dependency(name, kind=None, features=(), optional=False):
    return {"name": name, "kind": kind, "features": list(features), "optional": optional,
            "uses_default_features": False}


class Features(unittest.TestCase):
    def test_normal_and_test_features_are_unified(self):
        packages = {"app": {"features": {}, "dependencies": [dependency("player", features=["serde"]), dependency("player", "dev", ["synth"])]},
                    "player": {"features": {"serde": [], "synth": []}, "dependencies": []}}
        for tests, expected in ((False, ["serde"]), (True, ["serde", "synth"])):
            with self.subTest(tests=tests):
                features, _ = GENERATOR["resolve"](packages, {"app": []}, tests)
                self.assertEqual(features["player"], expected)

    def test_weak_feature_requires_the_optional_dependency(self):
        packages = {"app": {"features": {"default": ["library?/fast"], "enable": ["dep:library"]},
                            "dependencies": [dependency("library", optional=True)]},
                    "library": {"features": {"fast": []}, "dependencies": []}}
        for requested, expected in ((["default"], []), (["default", "enable"], ["fast"])):
            with self.subTest(requested=requested):
                features, _ = GENERATOR["resolve"](packages, {"app": requested})
                self.assertEqual(features["library"], expected)


class Platforms(unittest.TestCase):
    def test_unknown_platform_conditions_are_rejected(self):
        with self.assertRaisesRegex(ValueError, "target_arch"):
            GENERATOR["applies"]('cfg(target_arch = "aarch64")', "macos")

    def test_platform_conditions(self):
        cases = [
            ('cfg(any(target_os = "linux", target_os = "android"))', "android", True),
            ('cfg(any(target_os = "linux", target_os = "android"))', "macos", False),
            ('cfg(all(target_os = "linux", target_env = "gnu"))', "linux", True),
            ('cfg(all(target_os = "linux", target_env = "gnu"))', "android", False),
            ('cfg(unix)', "macos", True), ('cfg(unix)', "windows", False),
            ('cfg(not(target_os = "linux"))', "linux", False),
        ]
        for expression, system, expected in cases:
            with self.subTest(expression=expression, system=system):
                self.assertEqual(GENERATOR["applies"](expression, system), expected)


if __name__ == "__main__":
    unittest.main()
