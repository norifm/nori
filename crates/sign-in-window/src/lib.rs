//! A provider's sign-in page for a desktop client, in a window of its own: the client's own program run
//! again with `--sign-in`, so a client ships as one binary. The window ([`window`]) writes each page's
//! address and cookies to stdout; [`SignInWindow`] reads them in the client and hands over the cookies
//! once the provider says the account is signed in.

use std::io::BufRead;
use std::process::{Child, Command, Stdio};

use nori_core::providers::SignIn;
use nori_core::settings::ServerKind;

mod window;

/// The window, when this process was started as one: call first thing in `main`. Returns otherwise.
pub fn serve_if_asked() {
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() != Some("--sign-in") {
        return;
    }
    let (Some(title), Some(start)) = (args.next(), args.next()) else {
        eprintln!("usage: --sign-in TITLE ADDRESS");
        std::process::exit(2);
    };
    window::run(title, start);
}

/// A sign-in window open; closed when dropped.
pub struct SignInWindow(Child);

impl SignInWindow {
    /// Opens `kind`'s sign-in page titled `title`; `done` gets the window's [`SignInWindow::id`] and the
    /// signed-in session's cookies (a `Cookie` header's value), on a thread of its own. None where the
    /// provider has no page or no window can start.
    pub fn open(kind: ServerKind, title: String, done: impl FnOnce(u32, String) + Send + 'static) -> Option<SignInWindow> {
        let start = nori_core::providers::providers().into_iter().find(|i| i.kind == kind)?.sign_in.into_iter().find_map(|s| match s {
            SignIn::WebPage { start_url } => Some(start_url),
            SignIn::Paste => None,
        })?;
        let me = std::env::current_exe().ok()?;
        let mut child = Command::new(me).arg("--sign-in").arg(title).arg(start).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
        let (out, id) = (child.stdout.take()?, child.id());
        let watch = std::thread::Builder::new().name("nori-sign-in-window".into()).spawn(move || {
            for line in std::io::BufReader::new(out).lines().map_while(Result::ok) {
                let Some((address, cookies)) = line.split_once('\t') else { continue };
                if nori_core::providers::sign_in_done(kind, address.to_string(), cookies.to_string()) {
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
