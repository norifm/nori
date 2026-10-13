#!/usr/bin/env python3
"""Derive workspace Bazel targets and features from Cargo manifests."""
import argparse
import json
import pprint
import re
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SYSTEMS = ("macos", "linux", "android", "ios", "windows")


def applies(expression, system):
    if not expression:
        return True
    if not expression.startswith("cfg("):
        raise ValueError(f"unsupported Cargo condition: {expression}")
    tokens = iter(re.findall(r'"(?:[^"\\]|\\.)*"|\w+|[=(),]', expression))
    current = next(tokens, None)
    values = {"target_os": system, "target_env": "gnu" if system == "linux" else "msvc" if system == "windows" else "",
              "unix": system != "windows", "windows": system == "windows"}

    def parse():
        nonlocal current
        name = current
        current = next(tokens, None)
        if current == "=":
            value = json.loads(next(tokens))
            current = next(tokens, None)
            if name not in values:
                raise ValueError(f"unsupported Cargo condition: {name}")
            return values[name] == value
        if current != "(":
            if name not in values:
                raise ValueError(f"unsupported Cargo condition: {name}")
            return bool(values[name])
        current = next(tokens, None)
        arguments = []
        while current != ")":
            arguments.append(parse())
            if current == ",":
                current = next(tokens, None)
        current = next(tokens, None)
        if name == "any":
            return any(arguments)
        if name in ("all", "cfg"):
            return all(arguments)
        if name == "not":
            return not arguments[0]
        raise ValueError(f"unsupported Cargo condition: {name}")
    return parse()


def resolve(packages, roots, tests=False):
    features = {name: set() for name in packages}
    active = set(roots)
    dependencies = {name: set() for name in packages}
    for name, requested in roots.items():
        features[name].update(requested)
    changed = True
    while changed:
        before = (set(active), {n: set(v) for n, v in features.items()}, {n: set(v) for n, v in dependencies.items()})
        for name in list(active):
            package = packages[name]
            declared = {}
            for dep in package["dependencies"]:
                if tests or dep["kind"] != "dev":
                    declared.setdefault(dep.get("rename") or dep["name"], []).append(dep)
            dependencies[name].update(key for key, entries in declared.items() if any(not dep["optional"] for dep in entries))
            for feature in list(features[name]):
                for entry in package["features"].get(feature, []):
                    if entry.startswith("dep:"):
                        dependencies[name].add(entry[4:])
                    elif "/" in entry:
                        dependency, forwarded = entry.split("/", 1)
                        weak = dependency.endswith("?")
                        dependency = dependency.removesuffix("?")
                        if weak and dependency not in dependencies[name]:
                            continue
                        dependencies[name].add(dependency)
                        dep = declared[dependency][0]
                        if dep["name"] in packages:
                            features[dep["name"]].add(forwarded)
                    else:
                        features[name].add(entry)
            for key in dependencies[name]:
                if key not in declared:
                    continue
                for dep in declared[key]:
                    if dep["name"] in packages:
                        active.add(dep["name"])
                        features[dep["name"]].update(dep["features"])
                        if dep["uses_default_features"]:
                            features[dep["name"]].add("default")
        changed = before != (active, features, dependencies)
    return {name: sorted(value & packages[name]["features"].keys()) for name, value in features.items()}, dependencies


def write(path, contents, check):
    if path.exists() and path.read_text() == contents:
        return
    if check:
        raise SystemExit(f"{path.relative_to(ROOT)} is stale; run tools/bazel/generate.py")
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(contents)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    metadata = json.loads(subprocess.check_output(["cargo", "metadata", "--locked", "--all-features", "--format-version", "1"], cwd=ROOT))
    packages = {p["name"]: p for p in metadata["packages"] if p["id"] in metadata["workspace_members"]}
    resolved = {p["id"]: p for p in metadata["packages"]}
    nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
    profiles = {"tests": resolve(packages, {name: ["default"] for name in packages}, True)}
    for profile, names in (("desktop", ["nori-cli", "nori-desktop"]),):
        roots = {name: ["default"] for name in names}
        profiles[profile] = resolve(packages, roots)
        profiles[profile + "-tests"] = resolve(packages, roots, True)
    libraries = {name: next((t for t in p["targets"] if any(k in t["kind"] for k in ("lib", "proc-macro", "staticlib", "cdylib"))), None) for name, p in packages.items()}
    for name, package in packages.items():
        folder = Path(package["manifest_path"]).parent.relative_to(ROOT)
        local = []
        registry = []
        for dependency in package["dependencies"]:
            key = dependency.get("rename") or dependency["name"]
            active_profiles = [profile for profile, (_, deps) in profiles.items() if key in deps[name]]
            target = libraries.get(dependency["name"])
            if not target:
                entry = next(d for d in nodes[package["id"]]["deps"]
                             if resolved[d["pkg"]]["name"] == dependency["name"]
                             and (not dependency.get("rename") or d["name"] == key.replace("-", "_")))
                dep_package = resolved[entry["pkg"]]
                registry.append({"label": f"@crates//:{dep_package['name']}-{dep_package['version']}",
                                 "kind": dependency["kind"] or "normal",
                                 "macro": any("proc-macro" in t["kind"] for t in dep_package["targets"]),
                                 "systems": [system for system in SYSTEMS if applies(dependency["target"], system)],
                                 "profiles": active_profiles})
                continue
            dependency_folder = Path(packages[dependency["name"]]["manifest_path"]).parent.relative_to(ROOT)
            local.append({"package": dependency["name"], "label": f"//{dependency_folder}:{target['name']}", "alias": key.replace("-", "_"),
                          "kind": dependency["kind"] or "normal", "macro": "proc-macro" in target["kind"],
                          "profiles": active_profiles})
        resources = set()
        for source in (ROOT / folder).rglob("*.rs"):
            text = source.read_text()
            included_files = re.findall(r'include(?:_(?:bytes|str))?!\("([^"]+)"\)', text) + re.findall(r'#\[path\s*=\s*"([^"]+)"\]', text)
            for included in included_files:
                path = (source.parent / included).resolve()
                for other in packages.values():
                    other_folder = Path(other["manifest_path"]).parent
                    if other_folder != ROOT / folder and path.is_relative_to(other_folder):
                        target = str(path.relative_to(other_folder)) if path.suffix == ".rs" else "resources"
                        resources.add(f"//{other_folder.relative_to(ROOT)}:{target}")
        spec = {"name": name, "edition": package["edition"], "version": package["version"],
                "targets": [{"name": t["name"], "kind": t["kind"][0], "root": str(Path(t["src_path"]).relative_to(ROOT / folder)), "test": t["test"]} for t in package["targets"] if t["kind"][0] != "example"],
                "features": {profile: features[name] for profile, (features, _) in profiles.items()}, "local": local, "registry": registry,
                "resources": sorted(resources), "aliases": {d["label"]: d["alias"] for d in local if d["alias"] != d["label"].rsplit(":", 1)[1]}}
        contents = '# Generated from Cargo manifests by tools/bazel/generate.py.\nload("//tools/bazel:workspace.bzl", "workspace_crate")\n\nworkspace_crate(' + pprint.pformat(spec, sort_dicts=False, width=100) + ')\n'
        write(ROOT / folder / "BUILD.bazel", contents, args.check)
    members = [str(Path(p["manifest_path"]).parent.relative_to(ROOT)) for p in packages.values()]
    write(ROOT / "tools/bazel/members.bzl", '# Generated from Cargo manifests by tools/bazel/generate.py.\nMEMBERS = ' + pprint.pformat(sorted(members)) + '\n', args.check)


if __name__ == "__main__":
    main()
