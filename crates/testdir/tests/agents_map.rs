//! Checks that AGENTS.md's map matches the repository: every workspace crate has a row in its Layout
//! table, and every repository path it names exists.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn agents() -> String {
    fs::read_to_string(root().join("AGENTS.md")).expect("AGENTS.md")
}

/// The workspace members' directory names, from the root Cargo.toml.
fn members() -> BTreeSet<String> {
    let manifest = fs::read_to_string(root().join("Cargo.toml")).expect("Cargo.toml");
    let list = manifest.split_once("members = [").expect("members").1.split_once(']').expect("]").0;
    list.split(',')
        .filter_map(|m| m.trim().trim_matches('"').strip_prefix("crates/"))
        .map(str::to_owned)
        .collect()
}

/// The crate names in the first column of the Layout table.
fn mapped() -> BTreeSet<String> {
    let text = agents();
    let layout = text.split_once("## Layout").expect("## Layout").1;
    let layout = layout.split_once("\n## ").map_or(layout, |(section, _)| section);
    layout
        .lines()
        .filter_map(|line| line.strip_prefix("| "))
        .filter_map(|row| row.split_once(" |").map(|(first, _)| first))
        .filter(|first| !matches!(*first, "crate" | "---"))
        .flat_map(|first| first.split([',', '/']).map(|name| name.trim().to_owned()))
        .collect()
}

#[test]
fn every_workspace_crate_is_in_the_layout_table() {
    let (members, mapped) = (members(), mapped());
    let missing: Vec<_> = members.difference(&mapped).collect();
    let gone: Vec<_> = mapped.difference(&members).collect();
    assert!(missing.is_empty() && gone.is_empty(), "AGENTS.md Layout: missing {missing:?}, not a crate {gone:?}");
}

#[test]
fn paths_named_in_agents_md_exist() {
    const TOP: [&str; 8] = ["crates/", "tools/", "docs/", "app/", "core/", "ios/", "web/", "vendor/"];
    let text = agents();
    let missing: Vec<_> = text
        .split(['`', '(', ')', ' ', '\n', '\''])
        .map(|word| word.trim_end_matches(['.', ',', ':', ';']))
        .filter(|word| TOP.iter().any(|top| word.starts_with(top)))
        .filter(|word| !word.contains(['*', '<', '[', '{']) && !word.contains("..."))
        .filter(|word| !root().join(word).exists())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    assert!(missing.is_empty(), "AGENTS.md names paths that don't exist: {missing:?}");
}
