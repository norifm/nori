"""Workspace Rust rules populated from Cargo metadata."""
load("@rules_rust//cargo:defs.bzl", "cargo_build_script", "cargo_toml_env_vars")
load("@rules_rust//rust:defs.bzl", "rust_binary", "rust_library", "rust_proc_macro", "rust_shared_library", "rust_static_library", "rust_test")
load(":generated.bzl", "normalized_build_script")

def _features(spec):
    return select({"//tools/bazel:" + profile: values for profile, values in spec["features"].items()})

def _flags(spec):
    return select({
        "//tools/bazel:" + profile: ["-Copt-level=2", "-Cdebug-assertions=yes", "-Coverflow-checks=yes"] if profile == "tests" or profile.endswith("-tests") else []
        for profile in spec["features"]
    })

def _local(spec, kind, macro = False):
    kinds = [kind] if type(kind) == "string" else kind
    return select({
        "//tools/bazel:" + profile: sorted({d["label"]: True for d in spec["local"] if d["kind"] in kinds and d["macro"] == macro and profile in d["profiles"] and d["package"] != spec["name"]})
        for profile in spec["features"]
    })

def _registry(spec, kind, macro = False):
    kinds = [kind] if type(kind) == "string" else kind
    return select({
        "//tools/bazel:" + profile + "_" + system: sorted({d["label"]: True for d in spec["registry"] if d["kind"] in kinds and d["macro"] == macro and profile in d["profiles"] and system in d["systems"]})
        for profile in spec["features"]
        for system in ["macos", "linux", "android", "ios", "windows"]
    })

def workspace_crate(spec):
    native.package(default_visibility = ["//visibility:public"])
    native.filegroup(name = "files", srcs = native.glob(["**"], exclude = ["BUILD.bazel"]))
    native.filegroup(name = "resources", srcs = native.glob(["**"], exclude = ["**/*.rs", "BUILD.bazel"]))
    cargo_toml_env_vars(name = "cargo_env", src = "Cargo.toml", workspace = "//:Cargo.toml")
    features = _features(spec)
    data = [":resources"] + spec["resources"] + (["//:desktop_fonts"] if spec["name"] == "nori-desktop" else [])
    sources = native.glob(["src/**/*.rs"])
    build_script = next_target(spec, "custom-build")
    if build_script:
        normalized = spec["name"] in ["nori-android", "nori-desktop"]
        cargo_build_script(
            name = "raw_build_script" if normalized else "build_script",
            crate_root = build_script["root"],
            srcs = [build_script["root"]],
            edition = spec["edition"],
            pkg_name = spec["name"],
            version = spec["version"],
            crate_features = features,
            rustc_env_files = [":cargo_env"],
            build_script_env_files = [":cargo_env"],
            build_script_env = select({"//tools/bazel:" + profile: {"CARGO_CFG_FEATURE": ",".join(values)} for profile, values in spec["features"].items()}),
            data = [":files"] + (["//:workspace_sources"] if spec["name"] == "nori-android" else []) + (["//:desktop_fonts"] if spec["name"] == "nori-desktop" else []),
            deps = _registry(spec, "build") + _local(spec, "build"),
            proc_macro_deps = _registry(spec, "build", True) + _local(spec, "build", True),
        )
        if normalized:
            normalized_build_script(name = "build_script", script = ":raw_build_script", package = native.package_name(),
                                    data = [":files"] + (["//:desktop_fonts"] if spec["name"] == "nori-desktop" else []))
    common = dict(
        edition = spec["edition"],
        version = spec["version"],
        srcs = sources,
        crate_features = features,
        rustc_env = {"CARGO_MANIFEST_DIR": native.package_name()},
        rustc_env_files = [":cargo_env"],
        rustc_flags = _flags(spec),
        compile_data = data,
        aliases = spec["aliases"],
        deps = _registry(spec, "normal") + _local(spec, "normal") + ([":build_script"] if build_script else []),
        proc_macro_deps = _registry(spec, "normal", True) + _local(spec, "normal", True),
    )
    library = None
    for target in spec["targets"]:
        kind = target["kind"]
        if kind in ["custom-build", "test"]:
            continue
        rule = rust_binary if kind == "bin" else rust_proc_macro if kind == "proc-macro" else rust_library
        rule(name = target["name"], crate_root = target["root"], crate_name = target["name"].replace("-", "_"), **common)
        if kind in ["staticlib", "cdylib"]:
            linked = rust_static_library if kind == "staticlib" else rust_shared_library
            linked(name = "linked", crate_root = target["root"], crate_name = target["name"].replace("-", "_"), **common)
        if kind != "bin":
            library = target["name"]
        if target["test"]:
            rust_test(
                name = "unit_tests",
                crate = ":" + target["name"],
                rustc_flags = ["-Copt-level=2", "-Cdebug-assertions=yes", "-Coverflow-checks=yes"],
                data = ["//:test_data"] + (["//:kotlin_sources"] if spec["name"] == "nori-android" else []),
                aliases = spec["aliases"],
                deps = _registry(spec, "dev") + _local(spec, "dev"),
                proc_macro_deps = _registry(spec, "dev", True) + _local(spec, "dev", True),
            )
    for target in spec["targets"]:
        if target["kind"] != "test":
            continue
        rust_test(
            name = target["name"],
            crate_root = target["root"],
            crate_name = target["name"].replace("-", "_"),
            srcs = native.glob(["tests/**/*.rs"]),
            edition = spec["edition"],
            rustc_env = {"CARGO_MANIFEST_DIR": native.package_name()},
            rustc_env_files = [":cargo_env"],
            rustc_flags = ["-Copt-level=2", "-Cdebug-assertions=yes", "-Coverflow-checks=yes"],
            compile_data = data,
            data = ["//:test_data"] + (["//:workspace_sources"] if spec["name"] == "nori-testdir" and target["name"] == "registered" else []),
            aliases = spec["aliases"],
            deps = _registry(spec, ["normal", "dev"]) + _local(spec, ["normal", "dev"]) + ([":" + library] if library else []),
            proc_macro_deps = _registry(spec, ["normal", "dev"], True) + _local(spec, ["normal", "dev"], True),
        )

def next_target(spec, kind):
    for target in spec["targets"]:
        if target["kind"] == kind:
            return target
    return None
