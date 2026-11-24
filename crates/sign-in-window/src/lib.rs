//! A provider's sign-in page for a desktop client, in a window of its own: the client's own program run
//! again with `--sign-in`, so a client ships as one binary. The window ([`window`]) writes each page's
//! address and cookies to stdout; [`SignInWindow`] reads them in the client and hands over the cookies
//! once the provider says the account is signed in. The page goes through the profile's proxy, without
//! its login: no web view here takes one.

use std::io::BufRead;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;

use nori_core::providers::Kinds;
use nori_core::proxy::{proxy_address, proxy_from_form, SavedProxy};
use nori_core::settings::ServerKind;

mod window;

/// WebKitGTK's switch for its GPU-buffer renderer; set by the user, theirs stands.
#[cfg(target_os = "linux")]
const WEBKIT_DMABUF: &str = "WEBKIT_DISABLE_DMABUF_RENDERER";

/// The window, when this process was started as one: call first thing in `main`. Returns otherwise.
pub fn serve_if_asked() {
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() != Some("--sign-in") {
        return;
    }
    let (Some(title), Some(start)) = (args.next(), args.next()) else {
        eprintln!("usage: --sign-in TITLE ADDRESS [PROXY]");
        std::process::exit(2);
    };
    window::run(title, start, args.next().and_then(|p| proxy_from_form(p, String::new(), String::new()).ok().flatten()));
}

/// A sign-in window open; closed when dropped.
pub struct SignInWindow(Child);

/// Why no sign-in window opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoWindow {
    /// The provider has no page, or no window can start here.
    Unavailable,
    /// The profile has a proxy and this system's web view takes none: it signs in by pasting.
    ProxyUnsupported,
}

/// Whether a window through a proxy (`proxied`) is refused here: WKWebView takes a proxy from macOS 14
/// (wry's `mac-proxy`), and nori runs from macOS 11.
fn refused(proxied: bool) -> Option<NoWindow> {
    (cfg!(target_os = "macos") && proxied).then_some(NoWindow::ProxyUnsupported)
}

impl SignInWindow {
    /// Opens `kind`'s sign-in page (one of `kinds`) titled `title`, through `proxy` (the profile's);
    /// `done` gets the window's [`SignInWindow::id`] and the signed-in session's cookies (a `Cookie`
    /// header's value), on a thread of its own.
    pub fn open(kinds: Arc<Kinds>, kind: ServerKind, title: String, proxy: Option<SavedProxy>, done: impl FnOnce(u32, String) + Send + 'static) -> Result<SignInWindow, NoWindow> {
        let start = kinds.providers().into_iter().find(|i| i.kind == kind).and_then(|i| i.page().map(str::to_string)).ok_or(NoWindow::Unavailable)?;
        SignInWindow::open_at(kinds, kind, title, start, proxy, done)
    }

    /// [`SignInWindow::open`] at `start`: a page the provider's plugin asked for (`sign-in.open`).
    pub fn open_at(kinds: Arc<Kinds>, kind: ServerKind, title: String, start: String, proxy: Option<SavedProxy>, done: impl FnOnce(u32, String) + Send + 'static) -> Result<SignInWindow, NoWindow> {
        if let Some(why) = refused(proxy.is_some()) {
            return Err(why);
        }
        SignInWindow::start(kinds, kind, title, start, proxy, done).ok_or(NoWindow::Unavailable)
    }

    fn start(kinds: Arc<Kinds>, kind: ServerKind, title: String, start: String, proxy: Option<SavedProxy>, done: impl FnOnce(u32, String) + Send + 'static) -> Option<SignInWindow> {
        let me = std::env::current_exe().ok()?;
        let mut window = Command::new(me);
        // WebKitGTK hands the page's frames to the window as GPU buffers, which fail silently where its
        // graphics libraries do not fit the system's driver (an AppImage's bundled ones, NVIDIA's): the
        // window stays blank. Shared memory works with every driver, and a sign-in page costs nothing to draw.
        #[cfg(target_os = "linux")]
        if std::env::var_os(WEBKIT_DMABUF).is_none() {
            window.env(WEBKIT_DMABUF, "1");
        }
        window.arg("--sign-in").arg(title).arg(start);
        if let Some(proxy) = proxy {
            window.arg(proxy_address(Some(proxy)));
        }
        let mut child = window.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
        let (out, id) = (child.stdout.take()?, child.id());
        let watch = std::thread::Builder::new().name("nori-sign-in-window".into()).spawn(move || {
            for line in std::io::BufReader::new(out).lines().map_while(Result::ok) {
                let Some((address, cookies)) = line.split_once('\t') else { continue };
                if kinds.sign_in_done(kind.clone(), address.to_string(), cookies.to_string()) {
                    return done(id, cookies.to_string());
                }
            }
        });
        watch.ok()?;
        Some(SignInWindow(child))
    }

    pub fn id(&self) -> u32 {
        self.0.id()
    }

    /// Whether the window is still open.
    pub fn open_now(&mut self) -> bool {
        matches!(self.0.try_wait(), Ok(None))
    }
}

impl Drop for SignInWindow {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Only macOS, whose web view takes no proxy here, refuses a window through one; without a proxy no
    /// system does.
    #[test]
    fn a_proxy_is_refused_only_where_the_web_view_takes_none() {
        let proxied = if cfg!(target_os = "macos") { Some(NoWindow::ProxyUnsupported) } else { None };
        assert_eq!((refused(true), refused(false)), (proxied, None));
    }
}
