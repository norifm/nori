//! A repo's address as a user types it: a GitHub `user/repo`, or an index's URL. nori-plugins keeps its
//! `index.json` at the top of its default branch, so a `user/repo` is looked for there, then on GitHub
//! Pages.

/// The URLs the index `typed` names may be at, in the order to try them; none for no address.
pub fn candidates(typed: &str) -> Vec<String> {
    let typed = typed.trim().trim_end_matches('/');
    if typed.is_empty() || typed.contains(char::is_whitespace) {
        return Vec::new();
    }
    let (scheme, rest) = match typed.split_once("://") {
        Some((s, r)) if s.eq_ignore_ascii_case("https") || s.eq_ignore_ascii_case("http") => (s, r),
        Some(_) => return Vec::new(),
        None => ("https", typed),
    };
    let github = rest.strip_prefix("github.com/").or_else(|| rest.strip_prefix("www.github.com/"));
    let shorthand = !typed.contains("://") && is_shorthand(rest);
    if let Some(path) = github.or(shorthand.then_some(rest)) {
        let mut parts = path.split('/');
        return match (parts.next(), parts.next()) {
            (Some(user), Some(repo)) if !user.is_empty() && !repo.is_empty() => on_github(user, repo.trim_end_matches(".git")),
            _ => Vec::new(),
        };
    }
    let url = format!("{scheme}://{rest}");
    if url.to_ascii_lowercase().ends_with(".json") { vec![url] } else { vec![url.clone(), format!("{url}/index.json")] }
}

/// `user/repo`: two parts, the first with no dot, so a host's `example.com/nori` isn't one.
fn is_shorthand(text: &str) -> bool {
    let parts: Vec<&str> = text.split('/').collect();
    parts.len() == 2 && !parts[0].contains('.') && parts.iter().all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c)))
}

/// Where a GitHub repo's index may be: its `main`, its `master`, its Pages.
fn on_github(user: &str, repo: &str) -> Vec<String> {
    let raw = |branch: &str| format!("https://raw.githubusercontent.com/{user}/{repo}/{branch}/index.json");
    vec![raw("main"), raw("master"), format!("https://{}.github.io/{repo}/index.json", user.to_ascii_lowercase())]
}

/// A short name for the repo at index `url`: its repository's name for one on GitHub, else its host,
/// with no port.
pub fn short_name(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let mut parts = rest.split('/');
    let host = parts.next().unwrap_or(rest);
    let (first, second) = (parts.next().filter(|p| !p.is_empty()), parts.next().filter(|p| !p.is_empty()));
    let host = host.strip_prefix("www.").unwrap_or(host);
    match (host, first, second) {
        ("raw.githubusercontent.com" | "github.com", Some(_), Some(repo)) => repo.trim_end_matches(".git").to_string(),
        (h, Some(repo), _) if h.ends_with(".github.io") && !repo.ends_with(".json") => repo.to_string(),
        (h, _, _) => h.rsplit_once(':').filter(|(_, port)| port.chars().all(|c| c.is_ascii_digit())).map_or(h, |(h, _)| h).to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_typed_address_names_where_its_index_may_be() {
        let github = |u: &str, r: &str| on_github(u, r);
        let cases: [(&str, Vec<String>); 10] = [
            ("someone/nori-plugins", github("someone", "nori-plugins")),
            (" Someone/my.plugins/ ", github("Someone", "my.plugins")),
            ("github.com/someone/nori-plugins", github("someone", "nori-plugins")),
            ("https://github.com/someone/nori-plugins.git", github("someone", "nori-plugins")),
            ("https://github.com/someone/nori-plugins/tree/main", github("someone", "nori-plugins")),
            ("https://friend.example/nori/index.json", vec!["https://friend.example/nori/index.json".into()]),
            ("friend.example/nori", vec!["https://friend.example/nori".into(), "https://friend.example/nori/index.json".into()]),
            ("http://192.168.1.5:8000/", vec!["http://192.168.1.5:8000".into(), "http://192.168.1.5:8000/index.json".into()]),
            ("ftp://friend.example/index.json", Vec::new()),
            ("some one", Vec::new()),
        ];
        for (typed, want) in cases {
            assert_eq!(candidates(typed), want, "{typed}");
        }
        assert_eq!(
            candidates("norifm/nori-plugins")[0],
            crate::OFFICIAL,
            "nori's own is where a user/repo is looked for first"
        );
        assert_eq!(github("Someone", "p")[2], "https://someone.github.io/p/index.json");
    }

    #[test]
    fn a_repo_is_called_by_its_repositorys_name_else_its_host() {
        for (url, name) in [
            (crate::OFFICIAL, "nori-plugins"),
            ("https://github.com/someone/nori-dev-plugins.git", "nori-dev-plugins"),
            ("https://someone.github.io/plugins/index.json", "plugins"),
            ("https://someone.github.io/index.json", "someone.github.io"),
            ("https://www.friend.example/nori/index.json", "friend.example"),
            ("http://10.0.2.2:8000/index.json", "10.0.2.2"),
        ] {
            assert_eq!(short_name(url), name, "{url}");
        }
    }
}
