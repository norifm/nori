//! Checks that the device scripts start nori on this computer only through `host_peer`
//! (tools/e2e-lib.sh), which starts it on the silent output: a test never plays through the computer's
//! sound card.

use std::fs;
use std::path::Path;

/// A line that starts or builds a client to run: its binary, or cargo running or building it.
fn starts_client(line: &str) -> bool {
    let clients = ["nori-cli", "nori-desktop"];
    let binary = ["target/debug/", "target/release/"].iter().any(|dir| clients.iter().any(|c| line.contains(&format!("{dir}{c}"))));
    let cargo = line.contains("cargo run") || line.contains("cargo build");
    binary || cargo && clients.iter().any(|c| line.contains(&format!("-p {c}")))
}

#[test]
fn scripts_start_host_clients_only_on_the_silent_output() {
    let tools = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools");
    let mut found = Vec::new();
    for entry in fs::read_dir(&tools).expect("tools/") {
        let path = entry.expect("an entry").path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_owned();
        if !name.ends_with(".sh") || name == "e2e-lib.sh" {
            continue;
        }
        let text = fs::read_to_string(&path).expect("a script");
        found.extend(text.lines().enumerate().filter(|(_, l)| starts_client(l)).map(|(n, l)| format!("tools/{name}:{}: {}", n + 1, l.trim())));
    }
    assert!(found.is_empty(), "start nori on this computer with host_peer (tools/e2e-lib.sh):\n{}", found.join("\n"));
    let lib = fs::read_to_string(tools.join("e2e-lib.sh")).expect("tools/e2e-lib.sh");
    let starts: Vec<&str> = lib.lines().filter(|l| l.contains("tmux new-session")).collect();
    assert!(!starts.is_empty() && starts.iter().all(|l| l.contains("--silent-output")), "host_peer starts the client on the silent output: {starts:?}");
}

#[test]
fn a_client_started_or_built_to_run_is_seen() {
    let cases = [
        ("tmux new-session -d -s p \"$here/../target/debug/nori-cli --data d\"", true),
        ("cargo run --release -p nori-desktop", true),
        ("python3 with-resource.py build cargo build -j4 -p nori-cli >/dev/null", true),
        ("cargo test -j4 -p nori-cli", false),
        ("find \"$out\" -name 'nori-cli-*'", false),
    ];
    for (line, seen) in cases {
        assert_eq!(starts_client(line), seen, "{line}");
    }
}
