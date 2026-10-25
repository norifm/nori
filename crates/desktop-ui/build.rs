fn main() {
    // Only a few std widgets are used (text fields, scroll bars); cupertino suits both macOS and Linux. Its
    // light or dark palette follows `Theme.dark`.
    let config = slint_build::CompilerConfiguration::new().with_style("cupertino".into());
    slint_build::compile_with_config("ui/app.slint", config).expect("the interface compiles");
}
