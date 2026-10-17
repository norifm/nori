//! Signing in with a browser session's cookies: the jar, the `SAPISIDHASH` authorization music.youtube.com
//! asks of signed-in requests, and how the session's rotating cookies are kept up to date.
//!
//! Google rotates `__Secure-1PSIDTS`/`__Secure-3PSIDTS` (and the `SIDCC` family) and stops honoring a
//! session nobody rotates; whoever holds the cookies must ask for the rotation
//! ([`crate::InnerTube::rotate`]) and keep every `Set-Cookie` the answers carry. Two holders rotating one
//! session undo each other, so the cookies should come from a private window closed right after.

use std::time::Duration;

/// How often a session's cookies are rotated while in use (what browsers and other clients do).
pub const ROTATE_EVERY: Duration = Duration::from_secs(10 * 60);

/// The cookies of a youtube.com session, in the order they were given.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Cookies(Vec<(String, String)>);

impl Cookies {
    /// The cookies of `text`: a `Cookie` header's value (`a=1; b=2`, with or without `Cookie:` in front),
    /// or a Netscape `cookies.txt` export, whose youtube.com lines are taken.
    pub fn parse(text: &str) -> Cookies {
        let mut jar = Cookies::default();
        let netscape: Vec<&str> = text.lines().filter(|l| l.split('\t').count() == 7).collect();
        if !netscape.is_empty() {
            for line in netscape {
                let f: Vec<&str> = line.split('\t').collect();
                let domain = f[0].trim_start_matches("#HttpOnly_");
                if domain.trim_start_matches('.').ends_with("youtube.com") {
                    jar.set(f[5].trim(), f[6].trim());
                }
            }
            return jar;
        }
        let text = text.trim();
        let text = text.strip_prefix("Cookie:").or_else(|| text.strip_prefix("cookie:")).unwrap_or(text);
        for pair in text.split(';') {
            if let Some((k, v)) = pair.split_once('=') {
                jar.set(k.trim(), v.trim());
            }
        }
        jar
    }

    /// The value of cookie `name`.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.0.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    /// Sets `name`; whether its value changed.
    fn set(&mut self, name: &str, value: &str) -> bool {
        if name.is_empty() {
            return false;
        }
        match self.0.iter_mut().find(|(k, _)| k == name) {
            Some((_, v)) if v == value => false,
            Some((_, v)) => {
                *v = value.to_string();
                true
            }
            None => {
                self.0.push((name.to_string(), value.to_string()));
                true
            }
        }
    }

    /// The `Cookie` header's value.
    pub fn header(&self) -> String {
        self.0.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("; ")
    }

    /// Whether these are a signed-in session's: they hold an `SAPISID` to sign requests with.
    pub fn signed_in(&self) -> bool {
        self.sapisid().is_some()
    }

    fn sapisid(&self) -> Option<&str> {
        self.get("SAPISID").or_else(|| self.get("__Secure-3PAPISID")).filter(|v| !v.is_empty())
    }

    /// Takes in an answer's `Set-Cookie` values: new and changed cookies are kept, deleted ones dropped.
    /// Whether anything changed.
    pub fn merge(&mut self, set_cookies: &[String]) -> bool {
        let mut changed = false;
        for line in set_cookies {
            let mut parts = line.split(';');
            let Some((name, value)) = parts.next().and_then(|p| p.split_once('=')) else { continue };
            let (name, value) = (name.trim(), value.trim());
            let deleted = parts.map(str::trim).any(|a| {
                let a = a.to_ascii_lowercase();
                a.strip_prefix("max-age=").is_some_and(|n| n.trim().parse::<i64>().is_ok_and(|n| n <= 0)) || (a.starts_with("expires=") && a.contains("1970"))
            });
            if deleted {
                let before = self.0.len();
                self.0.retain(|(k, _)| k != name);
                changed |= self.0.len() != before;
            } else {
                changed |= self.set(name, value);
            }
        }
        changed
    }
}

/// The `Authorization` of a signed-in request from `origin` at `unix_secs`: `SAPISIDHASH`, with its
/// first- and third-party siblings when the session has their cookies (as yt-dlp sends them). None
/// without an `SAPISID`.
pub fn authorization(cookies: &Cookies, origin: &str, unix_secs: u64) -> Option<String> {
    let hash = |scheme: &str, sid: &str| format!("{scheme} {unix_secs}_{}", sha1_smol::Sha1::from(format!("{unix_secs} {sid} {origin}")).digest());
    let mut parts = vec![hash("SAPISIDHASH", cookies.sapisid()?)];
    if let Some(sid) = cookies.get("__Secure-1PAPISID") {
        parts.push(hash("SAPISID1PHASH", sid));
    }
    if let Some(sid) = cookies.get("__Secure-3PAPISID") {
        parts.push(hash("SAPISID3PHASH", sid));
    }
    Some(parts.join(" "))
}

/// The page a sign-in window opens: Google's sign-in for YouTube, going on to music.youtube.com once
/// signed in (as Metrolist and InnerTune open it in their WebViews).
pub const SIGN_IN_PAGE: &str = "https://accounts.google.com/ServiceLogin?service=youtube&continue=https%3A%2F%2Fmusic.youtube.com%2F";

/// Whether a sign-in window showing `address`, holding `cookies` for it (a `Cookie` header's value), is
/// done: back on music.youtube.com with a signed-in session's cookies.
pub fn signed_in_at(address: &str, cookies: &str) -> bool {
    let Some(rest) = address.strip_prefix("https://") else { return false };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let host = authority.split(':').next().unwrap_or_default();
    !authority.contains('@') && host.eq_ignore_ascii_case("music.youtube.com") && Cookies::parse(cookies).signed_in()
}

/// Whether a session last rotated at `rotated_at` (Unix seconds) is due again at `now`.
pub fn rotation_due(rotated_at: u64, now: u64) -> bool {
    now.saturating_sub(rotated_at) >= ROTATE_EVERY.as_secs() || now < rotated_at
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cookie_header_or_a_cookies_txt_is_read() {
        let c = Cookies::parse("Cookie: SID=a; __Secure-3PAPISID=b/c=; HSID=d");
        assert_eq!((c.get("SID"), c.get("__Secure-3PAPISID"), c.get("HSID")), (Some("a"), Some("b/c="), Some("d")));
        assert_eq!(c.header(), "SID=a; __Secure-3PAPISID=b/c=; HSID=d");
        assert!(c.signed_in());
        let txt = "# Netscape HTTP Cookie File\n.youtube.com\tTRUE\t/\tTRUE\t0\tSAPISID\tx\n#HttpOnly_.youtube.com\tTRUE\t/\tTRUE\t0\tSID\ty\n.google.com\tTRUE\t/\tTRUE\t0\tNID\tz\n";
        let c = Cookies::parse(txt);
        assert_eq!(c.header(), "SAPISID=x; SID=y");
        assert!(!Cookies::parse("VISITOR_INFO1_LIVE=v; YSC=w").signed_in(), "a visitor's cookies are no account");
    }

    /// Rotated cookies replace theirs, new ones join, deleted ones go; nothing new changes nothing.
    #[test]
    fn set_cookies_are_merged() {
        let mut c = Cookies::parse("SAPISID=s; __Secure-1PSIDTS=old; LOGIN_INFO=l");
        let rotated = vec![
            "__Secure-1PSIDTS=new; Domain=.youtube.com; Path=/; Expires=Sat, 17-Oct-2027 10:00:00 GMT; Secure; HttpOnly".to_string(),
            "__Secure-3PSIDCC=cc; Domain=.youtube.com; Path=/; Secure".to_string(),
            "LOGIN_INFO=; Domain=.youtube.com; Max-Age=0".to_string(),
        ];
        assert!(c.merge(&rotated));
        assert_eq!(c.header(), "SAPISID=s; __Secure-1PSIDTS=new; __Secure-3PSIDCC=cc");
        assert!(!c.merge(&rotated[..2]), "the same values again");
        assert!(c.merge(&["SAPISID=; expires=Thu, 01-Jan-1970 00:00:01 GMT".to_string()]));
        assert!(!c.signed_in());
    }

    /// The hash is sha1 of "<time> <SAPISID> <origin>", as music.youtube.com computes it.
    #[test]
    fn the_authorization_signs_with_each_sapisid() {
        let c = Cookies::parse("SAPISID=abc; __Secure-1PAPISID=one; __Secure-3PAPISID=three");
        let a = authorization(&c, "https://music.youtube.com", 1_700_000_000).unwrap();
        let expect = |scheme: &str, sid: &str| format!("{scheme} 1700000000_{}", sha1_smol::Sha1::from(format!("1700000000 {sid} https://music.youtube.com")).digest());
        assert_eq!(a, [expect("SAPISIDHASH", "abc"), expect("SAPISID1PHASH", "one"), expect("SAPISID3PHASH", "three")].join(" "));
        // sha1("1700000000 abc https://music.youtube.com"), worked out apart.
        assert!(a.starts_with("SAPISIDHASH 1700000000_2f3ec011e870f3fbd0238c090c2062c208cead32 "), "{a}");
        assert_eq!(authorization(&Cookies::parse("SID=x"), "o", 1), None);
    }

    /// Only music.youtube.com itself, reached with a session, ends a sign-in; Google's own pages, a
    /// look-alike host and a visitor's cookies do not.
    #[test]
    fn a_sign_in_ends_on_music_youtube_com_signed_in() {
        let session = "SID=a; __Secure-3PAPISID=b; HSID=c";
        for (address, cookies, done) in [
            ("https://music.youtube.com/", session, true),
            ("https://MUSIC.youtube.com:443/?cbrd=1", session, true),
            ("https://music.youtube.com", "VISITOR_INFO1_LIVE=v; YSC=w", false),
            ("https://accounts.google.com/ServiceLogin?continue=https://music.youtube.com/", session, false),
            ("https://music.youtube.com.example.org/", session, false),
            ("https://music.youtube.com@example.org/", session, false),
            ("http://music.youtube.com/", session, false),
            ("https://www.youtube.com/", session, false),
        ] {
            assert_eq!(signed_in_at(address, cookies), done, "{address}");
        }
    }

    #[test]
    fn rotation_is_due_every_ten_minutes() {
        assert!(!rotation_due(1000, 1000 + 599));
        assert!(rotation_due(1000, 1000 + 600));
        assert!(rotation_due(1000, 10), "a clock set back");
    }
}
