fn main() {
    // Only a few std widgets are used (text fields, scroll bars); cupertino-dark suits both macOS and Linux.
    let config = slint_build::CompilerConfiguration::new().with_style("cupertino-dark".into());
    slint_build::compile_with_config("ui/app.slint", config).expect("the interface compiles");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        glass_shaders();
    }
}

fn glass_shaders() {
    use naga::back::glsl::{Options, PipelineOptions, Version, Writer, WriterFlags};
    println!("cargo:rerun-if-changed=src/glass.wgsl");
    let source = std::fs::read_to_string("src/glass.wgsl").unwrap();
    let module = naga::front::wgsl::parse_str(&source).unwrap();
    let info = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&module).unwrap();
    let directory = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    for (version, suffix) in [(Version::Desktop(430), ""), (Version::Embedded { version: 300, is_webgl: false }, "_es")] {
        let options = Options { version, writer_flags: WriterFlags::empty(), ..Default::default() };
        for entry in &module.entry_points {
            let pipeline = PipelineOptions { shader_stage: entry.stage, entry_point: entry.name.clone(), multiview: None };
            let mut output = String::new();
            let reflection = Writer::new(&mut output, &module, &info, &options, &pipeline, Default::default()).unwrap().write().unwrap();
            std::fs::write(directory.join(format!("{}{suffix}.glsl", entry.name)), output).unwrap();
            if entry.stage == naga::ShaderStage::Vertex {
                let options = Options { writer_flags: WriterFlags::ADJUST_COORDINATE_SPACE, ..options.clone() };
                let mut output = String::new();
                Writer::new(&mut output, &module, &info, &options, &pipeline, Default::default()).unwrap().write().unwrap();
                std::fs::write(directory.join(format!("vs_texture{suffix}.glsl")), output).unwrap();
            }
            let mut bindings = String::new();
            let mut textures: Vec<_> = reflection.texture_mapping.into_iter().collect();
            textures.sort_by(|a, b| a.0.cmp(&b.0));
            for (name, mapping) in textures {
                let binding = module.global_variables[mapping.texture].binding.as_ref().unwrap().binding;
                bindings.push_str(&format!("({name:?}, {}),", if binding == 1 { 0 } else { 1 }));
            }
            std::fs::write(directory.join(format!("{}{suffix}.bindings", entry.name)), format!("&[{bindings}]")).unwrap();
        }
    }
}
