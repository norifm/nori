"""Make generated Rust resource paths independent of the producer's sandbox."""
load("@rules_rust//rust/private:providers.bzl", "BuildInfo")

def _normalize(ctx):
    original = ctx.attr.script[BuildInfo]
    output = ctx.actions.declare_directory(ctx.label.name + ".out_dir")
    ctx.actions.run_shell(
        command = 'python3 "$1" "$2" "$3" "$4"',
        arguments = [ctx.file._normalizer.path, original.out_dir.path, output.path, ctx.attr.package],
        inputs = depset([ctx.file._normalizer, original.out_dir], transitive = [original.compile_data, depset(ctx.files.data)]),
        outputs = [output],
        mnemonic = "NormalizeGeneratedRust",
    )
    return [DefaultInfo(files = depset([output])), BuildInfo(
        out_dir = output, rustc_env = original.rustc_env, dep_env = original.dep_env,
        flags = original.flags, linker_flags = original.linker_flags,
        link_search_paths = original.link_search_paths, compile_data = original.compile_data,
    )]

normalized_build_script = rule(
    implementation = _normalize,
    attrs = {"script": attr.label(providers = [BuildInfo]), "package": attr.string(),
             "data": attr.label_list(allow_files = True),
             "_normalizer": attr.label(default = ":normalize-generated.py", allow_single_file = True)},
)
