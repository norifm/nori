//! The pages of each list read a page at a time (`nori_core::paged::Paged`), by the token its first page
//! was read for: the app asks for more as the list nears its end, and this says whether a page goes out.

use std::collections::VecDeque;
use std::ffi::c_char;
use std::sync::Mutex;

use nori_core::paged::Paged;

use crate::session::c_text;

/// Lists whose pages are kept; older ones fall off. A global: the C entry points carry no handle.
static PAGED: Mutex<VecDeque<(u64, Paged<u32>)>> = Mutex::new(VecDeque::new());
const KEPT: usize = 24;

fn paged() -> std::sync::MutexGuard<'static, VecDeque<(u64, Paged<u32>)>> {
    PAGED.lock().unwrap_or_else(|e| e.into_inner())
}

/// List `token` is read from its first page, now on its way.
pub(crate) fn first(token: u64) {
    let mut lists = paged();
    lists.retain(|(t, _)| *t != token);
    let mut pages = Paged::new(0);
    pages.ask();
    lists.push_back((token, pages));
    while lists.len() > KEPT {
        lists.pop_front();
    }
}

fn with<T>(token: u64, f: impl FnOnce(&mut Paged<u32>) -> T) -> Option<T> {
    paged().iter_mut().find(|(t, _)| *t == token).map(|(_, p)| f(p))
}

/// The page of list `token` from `from` came, `next` after it: whether more may follow it.
pub(crate) fn answered(token: u64, from: u32, next: Option<u32>) -> bool {
    with(token, |p| {
        p.answered(&from, next);
        p.more()
    })
    .unwrap_or(false)
}

/// The page of list `token` from `from` could not be read; it is asked again with the next.
pub(crate) fn failed(token: u64, from: u32) {
    with(token, |p| p.failed(&from));
}

/// The list page `token` shows (`kind`, read with `arg` and no offset) nears its end: its next page is
/// read, answered on the page callback, unless one is on its way or the last is in.
///
/// # Safety
/// `arg` is NUL-terminated UTF-8, or NULL.
#[no_mangle]
pub unsafe extern "C" fn nori_ios_read_more(token: u64, kind: i32, arg: *const c_char) {
    let Some(offset) = with(token, Paged::ask).flatten() else { return };
    let arg = c_text(arg);
    let arg = match kind {
        crate::pages::PAGE_GENRE | crate::pages::PAGE_HOME_ROW => format!("{arg}\n{offset}"),
        _ => offset.to_string(),
    };
    nori_host::spawn("nori-ios-page", move || crate::pages::read(token, kind, arg));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_list_asks_its_pages_one_at_a_time() {
        first(7);
        assert_eq!(with(7, Paged::ask), Some(None), "the first page is on its way");
        assert!(answered(7, 0, Some(500)));
        assert_eq!(with(7, Paged::ask), Some(Some(500)));
        failed(7, 500);
        assert_eq!(with(7, Paged::ask), Some(Some(500)), "a failed page is asked again");
        assert!(!answered(7, 500, None));
        assert_eq!(with(7, Paged::ask), Some(None));
        assert!(!answered(8, 0, Some(1)), "a list not kept");
    }
}
