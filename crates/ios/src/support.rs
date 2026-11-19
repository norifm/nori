//! The Support page's C ABI: the stamp book and what to give (the core's `support_choice`), paid on
//! Stripe's checkout page in Safari (`nori_host::support::checkout`). What goes to the network answers as a
//! `REPORT_SUPPORT`.

use std::ffi::c_char;

use nori_core::support::{support_choice, support_currencies, support_first_choice, support_service, support_stamps, Cadence, SupportChoice, SupportPick, TypedAmount};
use nori_host::support::{currency_name, money, number};
use serde_json::{json, Value};

use crate::pages::owned;
use crate::session::{c_text, report, server, REPORT_SUPPORT};

/// `REPORT_SUPPORT`'s flags: what answered.
pub const SUPPORT_FAILED: i32 = 0;
pub const SUPPORT_GIVEN: i32 = 1;
pub const SUPPORT_REFRESHED: i32 = 2;
pub const SUPPORT_MANAGE: i32 = 3;

/// What is chosen as the page shows it: the offered amounts as numbers, the pick (-1 for Other), what is
/// typed, the limit it is past, and what Give gives.
fn chosen(c: &SupportChoice) -> Value {
    let (least, most) = match c.typed {
        Some(TypedAmount::TooLittle { least }) => (money(least, &c.currency), String::new()),
        Some(TypedAmount::TooMuch { most }) => (String::new(), money(most, &c.currency)),
        _ => (String::new(), String::new()),
    };
    json!({
        "currency": c.currency.code,
        "monthly": c.cadence == Cadence::Monthly,
        "offered": c.offered.iter().map(|a| number(*a, &c.currency)).collect::<Vec<_>>(),
        "pick": match c.pick { SupportPick::Offered { index } => index as i64, SupportPick::Other => -1 },
        "other": c.other,
        "least": least,
        "most": most,
        "amount": c.amount.map(|a| money(a, &c.currency)),
    })
}

fn choice_of(currency: *const c_char, monthly: i32, pick: i32, other: *const c_char) -> SupportChoice {
    let pick = if pick < 0 { SupportPick::Other } else { SupportPick::Offered { index: pick as u32 } };
    support_choice(c_text(currency), if monthly != 0 { Cadence::Monthly } else { Cadence::Once }, pick, c_text(other))
}

/// 1 when this build has a support service, so a Support page.
#[no_mangle]
pub extern "C" fn nori_ios_support_offered() -> i32 {
    support_service().is_some() as i32
}

/// The Support page as JSON to free, `null` for a build without a support service: the seals (each its
/// season, tilt and its season's accent, dark or light as `dark`), the season a payment made now is stamped
/// in (the local date as for `nori_ios_look`), whether a monthly payment runs, the currencies (code and
/// name) and what is first chosen in country `region`.
///
/// # Safety
/// `zone` and `region` are NUL-terminated UTF-8, or `NULL`.
#[no_mangle]
pub unsafe extern "C" fn nori_ios_support(unix_s: i64, offset_min: i32, zone: *const c_char, dark: i32, region: *const c_char) -> *mut c_char {
    if support_service().is_none() {
        return owned(&Value::Null);
    }
    let (month, day, southern) = crate::theme::local_date(unix_s, offset_min, &c_text(zone));
    let paid = crate::session::with_session(|s| s.core.session.settings.current().unwrap_or_default().support).unwrap_or_default();
    let seals: Vec<Value> = support_stamps(paid.paid, southern)
        .iter()
        .map(|s| json!({ "season": s.season as u8, "tilt": s.tilt, "accent": s.season.palette(dark != 0).accent }))
        .collect();
    owned(&json!({
        "seals": seals,
        "next": nori_look::theme::season_now(month, day, southern).season as u8,
        "running": paid.monthly,
        "currencies": support_currencies().iter().map(|c| json!({ "code": c.code, "name": currency_name(&c.code) })).collect::<Vec<_>>(),
        "first": chosen(&support_first_choice(c_text(region))),
    }))
}

/// `pick` (-1: Other) among `currency`'s amounts, `monthly` or once, `other` typed for Other: JSON to free.
///
/// # Safety
/// `currency` and `other` are NUL-terminated UTF-8, or `NULL`.
#[no_mangle]
pub unsafe extern "C" fn nori_ios_support_choice(currency: *const c_char, monthly: i32, pick: i32, other: *const c_char) -> *mut c_char {
    owned(&chosen(&choice_of(currency, monthly, pick, other)))
}

/// Starts paying what the choice comes to on Stripe's checkout page, stamped in the season of the local
/// date (as for `nori_ios_look`); a `REPORT_SUPPORT` answers with the page's address (`SUPPORT_GIVEN`) for
/// Safari, or why not (`SUPPORT_FAILED`: index 1 when a monthly payment runs in another currency).
///
/// # Safety
/// `currency`, `other` and `zone` are NUL-terminated UTF-8, or `NULL`.
#[no_mangle]
pub unsafe extern "C" fn nori_ios_support_give(currency: *const c_char, monthly: i32, pick: i32, other: *const c_char, unix_s: i64, offset_min: i32, zone: *const c_char) {
    let choice = choice_of(currency, monthly, pick, other);
    let (_, _, southern) = crate::theme::local_date(unix_s, offset_min, &c_text(zone));
    let Some((client, _)) = server() else { return };
    nori_host::spawn("nori-support-give", move || match nori_host::support::checkout(&client, &choice, southern) {
        Ok(Some(url)) => report(REPORT_SUPPORT, SUPPORT_GIVEN, 0, &url),
        Ok(None) => {}
        Err(e) => failed(e),
    });
}

/// Asks the support service what this app has paid; the settings keep it, and a `REPORT_SUPPORT`
/// (`SUPPORT_REFRESHED`) says so. Not reached, nothing is said.
#[no_mangle]
pub extern "C" fn nori_ios_support_refresh() {
    let Some((client, _)) = server() else { return };
    nori_host::spawn("nori-support-refresh", move || {
        if nori_core::transport::block_on(client.support_refresh()).is_ok() {
            report(REPORT_SUPPORT, SUPPORT_REFRESHED, 0, "");
        }
    });
}

/// Stripe's page where the monthly payment is changed or stopped: a `REPORT_SUPPORT` with its address
/// (`SUPPORT_MANAGE`), or why not.
#[no_mangle]
pub extern "C" fn nori_ios_support_manage() {
    let Some((client, _)) = server() else { return };
    nori_host::spawn("nori-support-manage", move || match nori_core::transport::block_on(client.support_manage()) {
        Ok(url) => report(REPORT_SUPPORT, SUPPORT_MANAGE, 0, &url),
        Err(e) => failed(e),
    });
}

fn failed(e: nori_core::transport::NetError) {
    let other_currency = matches!(e, nori_core::transport::NetError::Http { status: 409 });
    report(REPORT_SUPPORT, SUPPORT_FAILED, other_currency as i32, &e.to_string());
}

/// `text` as a QR code, JSON to free: `{ size, dark }`, `dark` a `0`/`1` per module, row by row; `null` when
/// it does not fit one.
///
/// # Safety
/// `text` is NUL-terminated UTF-8, or `NULL`.
#[no_mangle]
pub unsafe extern "C" fn nori_ios_qr(text: *const c_char) -> *mut c_char {
    owned(&nori_core::remote::qr_code(c_text(text)).map_or(Value::Null, |q| json!({ "size": q.size, "dark": q.dark.iter().map(|&d| if d { '1' } else { '0' }).collect::<String>() })))
}
