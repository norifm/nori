//! Writes the extension points from `wit/`: per exported interface a trait whose every call answers as
//! nori would on its own, `Nothing` implementing them all, the `Shim` that exports one plugin type's
//! points, and `plugin!`. The default follows the answer's type: `none`, "unsupported", empty, zero,
//! or what the call was handed when it hands back the same type (a stage's samples).

use std::fmt::Write as _;
use std::path::Path;

use wit_parser::{Function, Resolve, Type, TypeDefKind, TypeOwner, WorldItem};

fn main() {
    let wit = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../wit");
    println!("cargo:rerun-if-changed={}", wit.display());
    let mut resolve = Resolve::default();
    let (package, _) = resolve.push_dir(&wit).expect("wit/ parses");
    let world = resolve.select_world(&[package], Some("plugin")).expect("the plugin world");
    let exported: Vec<_> = resolve.worlds[world]
        .exports
        .values()
        .filter_map(|item| match item {
            WorldItem::Interface { id, .. } => Some(*id),
            _ => None,
        })
        .collect();
    let g = Gen { resolve: &resolve, exported: &exported };
    let points: Vec<(String, String)> = exported.iter().map(|&id| {
        let name = resolve.interfaces[id].name.clone().expect("named");
        (snake(&name), camel(&name))
    }).collect();
    let params: Vec<String> = (0..points.len()).map(|k| format!("P{k}")).collect();

    let mut out = String::new();
    for (k, &id) in exported.iter().enumerate() {
        let (module, name) = &points[k];
        let docs = resolve.interfaces[id].docs.contents.clone().unwrap_or_default();
        for line in docs.lines() {
            writeln!(out, "/// {line}").unwrap();
        }
        writeln!(out, "#[allow(unused_variables)]\npub trait {name} {{").unwrap();
        for f in resolve.interfaces[id].functions.values() {
            for line in f.docs.contents.as_deref().unwrap_or_default().lines() {
                writeln!(out, "    /// {line}").unwrap();
            }
            writeln!(out, "    fn {}(&mut self{}){} {{ {} }}", rust_name(&f.name), g.params(f), g.returns(f), g.default(f, module)).unwrap();
        }
        writeln!(out, "}}\nimpl {name} for Nothing {{}}\n").unwrap();

        let generics: Vec<String> = params.iter().enumerate().map(|(j, p)| if j == k { format!("{p}: {name} + Default + 'static") } else { p.clone() }).collect();
        writeln!(out, "impl<{}> exports::nori::plugin::{module}::Guest for Shim<{}> {{", generics.join(", "), params.join(", ")).unwrap();
        for f in resolve.interfaces[id].functions.values().filter(|f| !HAND_EXPORTED.contains(&f.name.as_str())) {
            let args: Vec<String> = f.params.iter().map(|(n, _)| rust_name(n)).collect();
            writeln!(out, "    fn {}({}){} {{ with::<P{k}, _>(|p| p.{}({})) }}", rust_name(&f.name), g.params(f).trim_start_matches(", "), g.returns(f), rust_name(&f.name), args.join(", ")).unwrap();
        }
        writeln!(out, "}}\n").unwrap();
    }

    writeln!(out, "/// The exported type: each extension point answered by its own type (the plugin's, or [`Nothing`]).").unwrap();
    writeln!(out, "// One type per extension point, so each answers with its own.\n#[allow(clippy::type_complexity)]\npub struct Shim<{0}>(std::marker::PhantomData<({0},)>);\n", params.join(", ")).unwrap();

    let names: Vec<&str> = points.iter().map(|(m, _)| m.as_str()).collect();
    writeln!(out, "/// Exports `$plugin` for the extension points named after it ({}); the others answer as nori would.", names.iter().map(|n| format!("`{n}`")).collect::<Vec<_>>().join(", ")).unwrap();
    writeln!(out, "#[macro_export]\nmacro_rules! plugin {{\n    ($plugin:ty, $manifest:expr $(, $point:ident)* $(,)?) => {{").unwrap();
    writeln!(out, "        const __NORI_LEN: usize = $crate::manifest::encoded_len(&$manifest, env!(\"CARGO_PKG_VERSION\"));").unwrap();
    writeln!(out, "        #[cfg_attr(target_arch = \"wasm32\", unsafe(link_section = \"nori:plugin\"))]\n        #[used]\n        static __NORI_MANIFEST: [u8; __NORI_LEN] = $crate::manifest::encode::<__NORI_LEN>(&$manifest, env!(\"CARGO_PKG_VERSION\"), 0 $(| $crate::__point_bit!($point))*);").unwrap();
    writeln!(out, "        type __NoriPlugin = $crate::Shim<").unwrap();
    for n in &names {
        writeln!(out, "            $crate::__pick!({n}, $plugin, [$($point)*]),").unwrap();
    }
    writeln!(out, "        >;\n        $crate::__export!(__NoriPlugin with_types_in $crate);").unwrap();
    writeln!(out, "        #[cfg(target_arch = \"wasm32\")]\n        #[unsafe(export_name = \"nori:plugin/sound-stage@2.0.0#process\")]\n        unsafe extern \"C\" fn __nori_stage_process(ptr: *mut f32, len: usize) -> *const u32 {{\n            unsafe {{ $crate::stage_abi::process::<$crate::__pick!(sound_stage, $plugin, [$($point)*])>(ptr, len) }}\n        }}").unwrap();
    writeln!(out, "        #[cfg(target_arch = \"wasm32\")]\n        #[unsafe(export_name = \"nori:plugin/sound-stage@2.0.0#save\")]\n        unsafe extern \"C\" fn __nori_stage_save() -> *const u32 {{\n            unsafe {{ $crate::stage_abi::save::<$crate::__pick!(sound_stage, $plugin, [$($point)*])>() }}\n        }}\n    }};\n}}\n").unwrap();
    writeln!(out, "/// The bit of extension point `$point` in the manifest, in `wit/`'s order.\n#[doc(hidden)]\n#[macro_export]\nmacro_rules! __point_bit {{").unwrap();
    for (k, n) in names.iter().enumerate() {
        writeln!(out, "    ({n}) => {{ 1u32 << {k} }};").unwrap();
    }
    writeln!(out, "}}\n").unwrap();
    writeln!(out, "/// `$plugin` when `$want` is among the named points, else [`Nothing`].\n#[doc(hidden)]\n#[macro_export]\nmacro_rules! __pick {{").unwrap();
    writeln!(out, "    ($want:ident, $plugin:ty, []) => {{ $crate::Nothing }};").unwrap();
    for n in &names {
        writeln!(out, "    ({n}, $plugin:ty, [{n} $($rest:ident)*]) => {{ $plugin }};").unwrap();
    }
    writeln!(out, "    ($want:ident, $plugin:ty, [$other:ident $($rest:ident)*]) => {{ $crate::__pick!($want, $plugin, [$($rest)*]) }};\n}}").unwrap();

    std::fs::write(Path::new(&std::env::var("OUT_DIR").unwrap()).join("points.rs"), out).unwrap();
}

/// Exported by `stage_abi` instead, without a post-return.
const HAND_EXPORTED: [&str; 2] = ["process", "save"];

struct Gen<'a> {
    resolve: &'a Resolve,
    exported: &'a [wit_parser::InterfaceId],
}

impl Gen<'_> {
    fn params(&self, f: &Function) -> String {
        f.params.iter().map(|(n, t)| format!(", {}: {}", rust_name(n), self.ty(t))).collect()
    }

    fn returns(&self, f: &Function) -> String {
        f.result.as_ref().map_or_else(String::new, |t| format!(" -> {}", self.ty(t)))
    }

    /// The answer nori would give itself.
    fn default(&self, f: &Function, module: &str) -> String {
        let Some(result) = &f.result else { return String::new() };
        let shown = self.ty(result);
        if let Some((n, _)) = f.params.iter().find(|(_, t)| self.ty(t) == shown) {
            return rust_name(n);
        }
        self.value(result).unwrap_or_else(|| panic!("{module}.{} answers {shown}, which has no answer of its own: make it option<...>", f.name))
    }

    fn value(&self, t: &Type) -> Option<String> {
        Some(match t {
            Type::Bool => "false".into(),
            Type::U8 | Type::U16 | Type::U32 | Type::U64 | Type::S8 | Type::S16 | Type::S32 | Type::S64 => "0".into(),
            Type::F32 | Type::F64 => "0.0".into(),
            Type::String => "String::new()".into(),
            Type::Id(id) => match &self.resolve.types[*id].kind {
                TypeDefKind::Option(_) => "None".into(),
                TypeDefKind::Result(_) => "Err(crate::unsupported())".into(),
                TypeDefKind::List(_) => "Vec::new()".into(),
                TypeDefKind::Tuple(t) => format!("({},)", t.types.iter().map(|t| self.value(t)).collect::<Option<Vec<_>>>()?.join(", ")),
                TypeDefKind::Type(t) => self.value(t)?,
                _ => return None,
            },
            _ => return None,
        })
    }

    /// `t` as wit-bindgen names it.
    fn ty(&self, t: &Type) -> String {
        match t {
            Type::Bool => "bool".into(),
            Type::U8 => "u8".into(),
            Type::U16 => "u16".into(),
            Type::U32 => "u32".into(),
            Type::U64 => "u64".into(),
            Type::S8 => "i8".into(),
            Type::S16 => "i16".into(),
            Type::S32 => "i32".into(),
            Type::S64 => "i64".into(),
            Type::F32 => "f32".into(),
            Type::F64 => "f64".into(),
            Type::Char => "char".into(),
            Type::String => "String".into(),
            Type::ErrorContext => panic!("no error-context in the plugin API"),
            Type::Id(id) => {
                let def = &self.resolve.types[*id];
                if let (Some(name), TypeOwner::Interface(owner)) = (&def.name, def.owner) {
                    let iface = snake(self.resolve.interfaces[owner].name.as_deref().expect("named"));
                    let side = if self.exported.contains(&owner) { "exports::nori::plugin" } else { "nori::plugin" };
                    return format!("crate::{side}::{iface}::{}", camel(name));
                }
                match &def.kind {
                    TypeDefKind::Option(t) => format!("Option<{}>", self.ty(t)),
                    TypeDefKind::Result(r) => format!("Result<{}, {}>", r.ok.as_ref().map_or("()".into(), |t| self.ty(t)), r.err.as_ref().map_or("()".into(), |t| self.ty(t))),
                    TypeDefKind::List(t) => format!("Vec<{}>", self.ty(t)),
                    TypeDefKind::Tuple(t) => format!("({},)", t.types.iter().map(|t| self.ty(t)).collect::<Vec<_>>().join(", ")),
                    TypeDefKind::Type(t) => self.ty(t),
                    k => panic!("{k:?} in an export"),
                }
            }
        }
    }
}

fn snake(name: &str) -> String {
    name.replace('-', "_")
}

fn camel(name: &str) -> String {
    name.split('-').map(|w| w[..1].to_uppercase() + &w[1..]).collect()
}

/// A WIT name as a Rust identifier: `in` is a keyword.
fn rust_name(name: &str) -> String {
    let n = snake(name);
    if matches!(n.as_str(), "in" | "type" | "move" | "match" | "loop" | "ref" | "static" | "self" | "use" | "where" | "async" | "await") {
        n + "_"
    } else {
        n
    }
}
