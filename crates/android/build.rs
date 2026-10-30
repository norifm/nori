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
    let mut scaffolding = boolean_answers(&cancellable_calls(&retain_async_objects(&lines.join("\n"))));
    // Beside the generator's own allows: it matches every error variant as `{ .. }`, whatever its fields,
    // and a foreign method's `Result` answer with a one-armed `match`.
    let module = scaffolding.find("mod uniffi_bindgen_kotlin_jni_scaffolding {").expect("the generated scaffolding is one module");
    scaffolding.insert_str(module, "#[allow(clippy::unneeded_struct_pattern, clippy::single_match)]\n");
    let end = scaffolding.rfind('}').expect("the generated scaffolding closes its module");
    scaffolding.insert_str(end, "include!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/src/kotlin_calls.rs\"));\n");
    let end = scaffolding.rfind('}').expect("the generated scaffolding closes its module");
    scaffolding.insert_str(end, "#[cfg(test)] mod receiver_tests { include!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/src/receiver_tests.rs\")); }\n");
    assert!(!scaffolding.contains("JNI_OnLoad"), "the generated scaffolding has a second JNI_OnLoad");
    std::fs::write(&path, scaffolding).expect("the scaffolding is writable");

    // The scaffolding is generated from these crates' sources, which cargo does not watch for us.
    println!("cargo:rerun-if-changed=build.rs");
    for krate in ["core", "model", "db", "net", "library", "automix", "settings", "lyrics", "devices", "queue", "transfers", "perf", "remote", "look", "plugins", "backup"] {
        for input in ["src", "Cargo.toml", "uniffi.toml"] {
            println!("cargo:rerun-if-changed=../{krate}/{input}");
        }
    }
}

// NORI: a Rust future is dropped when Kotlin frees it, not once the last waker lets go, so a call Kotlin
// cancelled stops at once; an async Kotlin call it was waiting for is cancelled with it (kotlin_calls.rs).
fn cancellable_calls(scaffolding: &str) -> String {
    const FREE: &str = "::std::sync::Arc::decrement_strong_count(ptr);";
    const SENDER: &str = "uniffi_sender.into_raw().expose_provenance() as ::std::primitive::i64";
    const ATTACH: &str = "uniffi_jni::attach_current_thread(uniffi_get_global_jvm(), |uniffi_env| -> uniffi::Result<()> {";
    let (mut freed, mut calls) = (0, 0);
    let out = scaffolding.split("#[unsafe(no_mangle)]").map(|function| {
        if function.contains("fn Java_uniffi_Scaffolding_rustFutureFree") {
            assert_eq!(function.matches(FREE).count(), 1, "a generated future free drops one handle");
            freed += 1;
            return function.replacen(FREE, &format!("let uniffi_dropped = ::std::mem::replace(&mut *(*ptr).future.lock().unwrap(), ::std::boxed::Box::pin(::std::future::pending()));\n        drop(uniffi_dropped);\n        {FREE}"), 1);
        }
        function.split("let (uniffi_sender, uniffi_receiver)").enumerate().map(|(i, part)| {
            if i == 0 { return part.to_string(); }
            assert!(part.contains(ATTACH) && part.contains(SENDER) && part.contains("uniffi_receiver.await"), "a generated async Kotlin call has the expected shape");
            calls += 1;
            part.replacen(SENDER, "uniffi_oneshot", 1)
                .replacen(ATTACH, &format!("let uniffi_oneshot = {SENDER};\n        {ATTACH}"), 1)
                .replacen("uniffi_receiver.await", "UniffiKotlinCall { oneshot: uniffi_oneshot, answer: uniffi_receiver, answered: false }.await", 1)
        }).collect::<Vec<_>>().join("let (uniffi_sender, uniffi_receiver)")
    }).collect::<Vec<_>>().join("#[unsafe(no_mangle)]");
    assert!(freed > 0 && calls > 0, "the generated scaffolding frees futures and makes async Kotlin calls");
    out
}

// NORI: the generator calls a foreign method answering a bool as one answering a byte (`call_byte`, its
// `i8` handed to `lift_bool`), which doesn't build; it is called as the boolean method its descriptor says.
fn boolean_answers(scaffolding: &str) -> String {
    const DISPATCH: &str = "unsafe fn callback_interface_dispatch_";
    scaffolding.split(DISPATCH).enumerate().map(|(i, part)| {
        let end = part.find("\n}\n").unwrap_or(part.len());
        let (body, rest) = part.split_at(end);
        if i == 0 || !body.contains(") -> uniffi::Result<::std::primitive::bool> {") {
            return part.to_string();
        }
        assert!(body.contains(")Z\",") && body.contains("METHOD.call_byte(") && body.contains("lift_bool(uniffi_env, uniffi_return)"), "a generated foreign bool answer has the expected shape");
        body.replacen("METHOD.call_byte(", "METHOD.call_boolean(", 1) + rest
    }).collect::<Vec<_>>().join(DISPATCH)
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
