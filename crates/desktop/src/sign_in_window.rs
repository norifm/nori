//! `nori-desktop --sign-in <title> <address>`: a window showing a provider's own sign-in page in the
//! system's web view. After every page it writes the page's address, a tab and its cookies (a `Cookie`
//! header's value) as one line to stdout; the app decides from those when the account is signed in
//! (`sign_in_done`) and ends this process. A process of its own: the web view's event loop (tao, GTK on
//! Linux) and the app's (winit) would each want the main thread.
//!
//! The session is the web view's private one (incognito): nothing is kept after the window closes, so it
//! cannot rotate the account's cookies behind the app's back.

use std::io::Write;

use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::window::WindowBuilder;
use wry::{PageLoadEvent, WebViewBuilder};

/// Runs the window for `title`, starting at `start`, until it is closed or the app ends it.
pub fn run(title: String, start: String) -> ! {
    let event_loop = EventLoopBuilder::<String>::with_user_event().build();
    let window = match WindowBuilder::new().with_title(title).with_inner_size(tao::dpi::LogicalSize::new(480.0, 720.0)).build(&event_loop) {
        Ok(w) => w,
        Err(e) => fail(&format!("no window: {e}")),
    };
    let loaded = event_loop.create_proxy();
    let builder = WebViewBuilder::new()
        .with_url(start)
        .with_incognito(true)
        // Web pages only: no app links or other schemes.
        .with_navigation_handler(|address| address.starts_with("https://"))
        .with_on_page_load_handler(move |event, address| {
            if matches!(event, PageLoadEvent::Finished) {
                let _ = loaded.send_event(address);
            }
        });
    #[cfg(not(any(target_os = "linux", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd", target_os = "dragonfly")))]
    let webview = builder.build(&window);
    #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd", target_os = "dragonfly"))]
    let webview = {
        use tao::platform::unix::WindowExtUnix;
        use wry::WebViewBuilderExtUnix;
        match window.default_vbox() {
            Some(vbox) => builder.build_gtk(vbox),
            None => fail("no window box"),
        }
    };
    let webview = match webview {
        Ok(w) => w,
        Err(e) => fail(&format!("no web view: {e}")),
    };
    event_loop.run(move |event, _, flow| {
        *flow = ControlFlow::Wait;
        match event {
            Event::UserEvent(address) => {
                // wry's cookies_for_url matches the host exactly, missing a session's `.youtube.com` cookies.
                let header = cookie_header(&webview.cookies().unwrap_or_default(), &address);
                let mut out = std::io::stdout().lock();
                // The app gone: nobody to sign in for.
                if writeln!(out, "{address}\t{header}").and_then(|_| out.flush()).is_err() {
                    *flow = ControlFlow::Exit;
                }
            }
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => *flow = ControlFlow::Exit,
            _ => {}
        }
    });
}

/// The `Cookie` header a browser sends to `address`: the cookies whose domain is its host or a domain above
/// it, whose path is above its path, and, secure ones, only over https.
fn cookie_header(cookies: &[wry::cookie::Cookie<'static>], address: &str) -> String {
    let (https, rest) = match address.split_once("://") {
        Some((scheme, rest)) => (scheme == "https", rest),
        None => (false, address),
    };
    let (host, path) = rest.split_at(rest.find(['/', '?', '#']).unwrap_or(rest.len()));
    let host = host.rsplit_once('@').map_or(host, |(_, h)| h);
    let host = host.split(':').next().unwrap_or_default().to_ascii_lowercase();
    let path = if path.starts_with('/') { path } else { "/" };
    cookies
        .iter()
        .filter(|c| {
            let domain = c.domain().unwrap_or_default().trim_start_matches('.').to_ascii_lowercase();
            let in_domain = host == domain || host.ends_with(&format!(".{domain}"));
            let in_path = path.starts_with(c.path().unwrap_or("/"));
            in_domain && in_path && (https || !c.secure().unwrap_or(false))
        })
        .map(|c| format!("{}={}", c.name(), c.value()))
        .collect::<Vec<_>>()
        .join("; ")
}

fn fail(why: &str) -> ! {
    eprintln!("nori sign-in window: {why}");
    std::process::exit(1);
}

#[cfg(test)]
mod tests {
    use super::*;
    use wry::cookie::Cookie;

    #[test]
    fn a_session_is_sent_to_its_subdomains() {
        let cookie = |name: &'static str, domain: &'static str, secure: bool| Cookie::build((name, "v")).domain(domain).path("/").secure(secure).build();
        let jar = [cookie("SID", ".youtube.com", false), cookie("SAPISID", ".youtube.com", true), cookie("NID", ".google.com", false), cookie("x", "music.youtube.com", false)];
        assert_eq!(cookie_header(&jar, "https://music.youtube.com/?a=1"), "SID=v; SAPISID=v; x=v");
        assert_eq!(cookie_header(&jar, "http://music.youtube.com/"), "SID=v; x=v", "secure ones over https only");
        assert_eq!(cookie_header(&jar, "https://notyoutube.com/"), "");
    }
}
