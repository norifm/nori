use std::path::PathBuf;

fn main() {
    // The uniffi scaffolding as `Java_uniffi_Scaffolding_*` JNI functions.
    uniffi_bindgen_kotlin_jni::generate_scaffolding();

    // A library has one `JNI_OnLoad` and ours registers the natives, so the generated one is renamed and
    // called from ours. A uniffi revision that generates it differently fails the build here.
    let path = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("uniffi_bindgen_kotlin_jni.uniffi.rs");
    let scaffolding = std::fs::read_to_string(&path).expect("the scaffolding was generated");
    let mut lines: Vec<&str> = scaffolding.lines().collect();
    let on_load = lines.iter().position(|l| l.trim_start().starts_with("unsafe extern \"system\" fn JNI_OnLoad(")).expect("the generated scaffolding has a JNI_OnLoad");
    assert!(on_load > 0 && lines[on_load - 1].trim() == "#[unsafe(no_mangle)]", "the generated JNI_OnLoad is exported as expected");
    let renamed = lines[on_load].replacen("unsafe extern \"system\" fn JNI_OnLoad(", "pub(crate) unsafe extern \"system\" fn uniffi_on_load(", 1);
    lines[on_load] = &renamed;
    lines.remove(on_load - 1);
    let mut scaffolding = retain_async_objects(&lines.join("\n"));
    let end = scaffolding.rfind('}').expect("the generated scaffolding closes its module");
    scaffolding.insert_str(end, "#[cfg(test)] mod receiver_tests { include!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/src/receiver_tests.rs\")); }\n");
    assert!(!scaffolding.contains("JNI_OnLoad"), "the generated scaffolding has a second JNI_OnLoad");
    std::fs::write(&path, scaffolding).expect("the scaffolding is writable");

    // The scaffolding is generated from these crates' sources, which cargo does not watch for us.
    println!("cargo:rerun-if-changed=build.rs");
    for krate in ["core", "model", "db", "net", "library", "automix", "settings", "lyrics", "devices", "queue", "transfers", "perf", "remote", "look"] {
        for input in ["src", "Cargo.toml", "uniffi.toml"] {
            println!("cargo:rerun-if-changed=../{krate}/{input}");
        }
    }
}

// NORI: borrowed JNI objects need owned handles while an async call is outstanding.
fn retain_async_objects(scaffolding: &str) -> String {
    scaffolding.split("#[unsafe(no_mangle)]").map(|function| {
        if !function.contains("let uniffi_future = async move {") {
            return function.to_string();
        }
        let mut clone = None;
        function.split_inclusive('\n').map(|line| {
            if let Some((before, called)) = line.split_once(" = lift_object_") {
                let id = called.strip_prefix("receiver_ref_").or_else(|| called.strip_prefix("ref_"));
                if let Some(id) = id.and_then(|v| v.strip_suffix("(\n")) {
                    assert!(id.chars().all(|c| c.is_ascii_digit()), "the generated object type has a numeric id");
                    clone = Some(id);
                    return format!("{before} = lift_type_{id}(\n");
                }
            }
            if line.trim_start().starts_with("r#uniffi_arg_") {
                if let Some(id) = clone.take() {
                    let indent = &line[..line.len() - line.trim_start().len()];
                    let argument = line.trim().strip_suffix(',').expect("the generated object argument ends with a comma");
                    return format!("{indent}Java_uniffi_Scaffolding_objectCloneHandle{id}(uniffi_env, ::std::ptr::null_mut(), {argument}),\n");
                }
            }
            line.to_string()
        }).collect::<String>()
    }).collect::<Vec<_>>().join("#[unsafe(no_mangle)]")
}
