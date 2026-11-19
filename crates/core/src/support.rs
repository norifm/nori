//! Supporting nori: the amounts offered in each currency, the support service a build talks to (a
//! Cloudflare Worker over Stripe, `support/worker`), and the stamp book, one seal per payment.
//!
//! The service's address and Stripe's publishable key come from the build's environment
//! (`NORI_SUPPORT_URL`, `NORI_STRIPE_KEY`); a build without them has no support page.

use serde::Deserialize;

use crate::client::{Client, NetResult};
use crate::settings::{SettingChange, Supporter};
use crate::transport::{Exchange, NetError};
use nori_look::theme::Season;

const SERVICE_URL: Option<&str> = option_env!("NORI_SUPPORT_URL");
const PUBLISHABLE_KEY: Option<&str> = option_env!("NORI_STRIPE_KEY");
const TIMEOUT_MS: u32 = 20_000;

/// One payment or one payment a month.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum Cadence {
    Once,
    Monthly,
}

/// How a client takes the payment: Stripe's own sheet or form inside the app (Android, the web), or
/// Stripe's checkout page in the browser (desktop, terminal, iPod).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum PayIn {
    App,
    Browser,
}

/// A currency support is offered in: its ISO code, how many digits its minor unit has, and the amounts
/// offered for each cadence, in minor units (any other is typed).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct SupportCurrency {
    pub code: String,
    pub digits: u8,
    pub once: Vec<u64>,
    pub monthly: Vec<u64>,
    /// The least and most one payment may be.
    pub least: u64,
    pub most: u64,
}

/// A currency's amounts in whole units: what [`SupportCurrency`] gives in minor ones.
struct Offered {
    code: &'static str,
    digits: u8,
    once: [u64; 5],
    monthly: [u64; 5],
    least: u64,
    most: u64,
}

const fn offered(code: &'static str, digits: u8, once: [u64; 5], monthly: [u64; 5], least: u64, most: u64) -> Offered {
    Offered { code, digits, once, monthly, least, most }
}

/// In the order offered.
const CURRENCIES: &[Offered] = &[
    offered("EUR", 2, [5, 10, 20, 30, 50], [3, 5, 10, 15, 25], 1, 5_000),
    offered("USD", 2, [5, 10, 20, 30, 50], [3, 5, 10, 15, 25], 1, 5_000),
    offered("GBP", 2, [5, 10, 20, 30, 50], [3, 5, 10, 15, 25], 1, 5_000),
    offered("PLN", 2, [20, 40, 80, 125, 250], [10, 20, 40, 60, 100], 5, 20_000),
    offered("CHF", 2, [5, 10, 20, 30, 50], [3, 5, 10, 15, 25], 1, 5_000),
    offered("CZK", 2, [100, 250, 500, 750, 1_000], [75, 125, 250, 350, 500], 25, 100_000),
    offered("SEK", 2, [50, 100, 200, 300, 500], [30, 50, 100, 150, 250], 10, 50_000),
    offered("NOK", 2, [50, 100, 200, 300, 500], [30, 50, 100, 150, 250], 10, 50_000),
    offered("DKK", 2, [40, 75, 150, 225, 375], [25, 40, 75, 110, 190], 10, 40_000),
    offered("CAD", 2, [5, 10, 20, 30, 50], [3, 5, 10, 15, 25], 1, 5_000),
    offered("AUD", 2, [5, 10, 20, 30, 50], [3, 5, 10, 15, 25], 1, 5_000),
    offered("JPY", 0, [500, 1_000, 2_000, 3_000, 5_000], [300, 500, 1_000, 1_500, 2_500], 100, 500_000),
];

/// Countries outside the euro area whose own currency is offered, by ISO 3166 code.
const OWN_CURRENCY: &[(&str, &str)] = &[
    ("US", "USD"), ("GB", "GBP"), ("PL", "PLN"), ("CH", "CHF"), ("LI", "CHF"), ("CZ", "CZK"), ("SE", "SEK"), ("NO", "NOK"),
    ("DK", "DKK"), ("CA", "CAD"), ("AU", "AUD"), ("JP", "JPY"),
];

/// The euro area, by ISO 3166 code.
const EURO: &[&str] = &[
    "AT", "BE", "BG", "HR", "CY", "EE", "FI", "FR", "DE", "GR", "IE", "IT", "LV", "LT", "LU", "MT", "NL", "PT", "SK", "SI", "ES",
    "AD", "MC", "SM", "VA", "ME", "XK",
];

/// The currencies support is offered in, the most common first.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn support_currencies() -> Vec<SupportCurrency> {
    CURRENCIES
        .iter()
        .map(|o| {
            let minor = |units: u64| units * 10u64.pow(o.digits as u32);
            SupportCurrency {
                code: o.code.to_string(),
                digits: o.digits,
                once: o.once.map(minor).to_vec(),
                monthly: o.monthly.map(minor).to_vec(),
                least: minor(o.least),
                most: minor(o.most),
            }
        })
        .collect()
}

/// The currency first offered in country `region` (ISO 3166, as the phone's locale has it): its own
/// when offered, the euro in the euro area, else the dollar.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn support_currency_for(region: String) -> String {
    let region = region.to_ascii_uppercase();
    match OWN_CURRENCY.iter().find(|(r, _)| *r == region) {
        Some((_, code)) => code.to_string(),
        None if EURO.contains(&region.as_str()) => "EUR".to_string(),
        None => "USD".to_string(),
    }
}

/// What a typed amount comes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum TypedAmount {
    /// Nothing typed, or not a number.
    Unreadable,
    /// In minor units, within what a payment may be.
    Fine { minor: u64 },
    TooLittle { least: u64 },
    TooMuch { most: u64 },
}

/// `typed` in `currency`'s minor units: digits with at most one decimal point or comma and no more
/// decimals than the currency has; spaces and the grouping a keyboard leaves are ignored.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn support_typed(currency: String, typed: String) -> TypedAmount {
    let Some(c) = support_currencies().into_iter().find(|c| c.code == currency) else { return TypedAmount::Unreadable };
    let text: String = typed.chars().filter(|ch| !ch.is_whitespace() && *ch != '\'' && *ch != '\u{a0}').collect();
    let (whole, part) = match text.rsplit_once(['.', ',']) {
        Some((w, p)) => (w, p),
        None => (text.as_str(), ""),
    };
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if !digits(whole) || (!part.is_empty() && !digits(part)) || part.len() > c.digits as usize || whole.len() > 12 {
        return TypedAmount::Unreadable;
    }
    let scale = 10u64.pow(c.digits as u32);
    let fraction = if part.is_empty() { 0 } else { part.parse::<u64>().unwrap_or(0) * 10u64.pow((c.digits as usize - part.len()) as u32) };
    let minor = whole.parse::<u64>().unwrap_or(0) * scale + fraction;
    if minor < c.least {
        TypedAmount::TooLittle { least: c.least }
    } else if minor > c.most {
        TypedAmount::TooMuch { most: c.most }
    } else {
        TypedAmount::Fine { minor }
    }
}

/// Which amount is chosen: one of the amounts offered, or Other, what is typed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum SupportPick {
    Offered { index: u32 },
    Other,
}

/// What the page has chosen and what it comes to: the amounts offered in the currency for the cadence,
/// what is typed for Other (kept while an offered amount is chosen), how that reads when Other is
/// chosen, and the amount to pay in minor units, None while nothing payable is chosen.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct SupportChoice {
    pub currency: SupportCurrency,
    pub cadence: Cadence,
    pub pick: SupportPick,
    pub offered: Vec<u64>,
    pub other: String,
    pub typed: Option<TypedAmount>,
    pub amount: Option<u64>,
}

/// `pick` among `currency`'s amounts for `cadence`, with `other` typed for Other, and what it comes to; an
/// unknown currency is the first offered.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn support_choice(currency: String, cadence: Cadence, pick: SupportPick, other: String) -> SupportChoice {
    let all = support_currencies();
    let currency = all.iter().find(|c| c.code == currency).unwrap_or(&all[0]).clone();
    let offered = match cadence {
        Cadence::Once => currency.once.clone(),
        Cadence::Monthly => currency.monthly.clone(),
    };
    let (typed, amount) = match pick {
        SupportPick::Offered { index } => (None, offered.get(index as usize).copied()),
        SupportPick::Other => {
            let t = support_typed(currency.code.clone(), other.clone());
            (Some(t), match t {
                TypedAmount::Fine { minor } => Some(minor),
                _ => None,
            })
        }
    };
    SupportChoice { currency, cadence, pick, offered, other, typed, amount }
}

/// What the page first offers in country `region`: its currency, once, the second amount.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn support_first_choice(region: String) -> SupportChoice {
    support_choice(support_currency_for(region), Cadence::Once, SupportPick::Offered { index: 1 }, String::new())
}

/// The support service this build talks to; None for a build without one, which offers no support page.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct SupportService {
    /// Stripe's publishable key, for the payment sheet or form.
    pub publishable_key: String,
    /// Stripe's test mode: nothing is charged, and Google Pay is its test environment.
    pub test: bool,
    /// The country nori is paid in (ISO 3166), as Google Pay asks.
    pub merchant_country: String,
}

/// Where the Stripe account nori is paid through is registered.
const MERCHANT_COUNTRY: &str = "PL";

#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn support_service() -> Option<SupportService> {
    let (Some(url), Some(key)) = (SERVICE_URL, PUBLISHABLE_KEY) else { return None };
    (!url.is_empty() && !key.is_empty()).then(|| SupportService { publishable_key: key.to_string(), test: key.starts_with("pk_test_"), merchant_country: MERCHANT_COUNTRY.to_string() })
}

/// One seal in the stamp book: when it was paid, the season it was paid in and how far it is turned.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct Stamp {
    pub paid_s: i64,
    pub season: Season,
    /// Degrees, as a hand stamps it: a few either way, the same for the same payment on every client.
    pub tilt: f32,
}

/// The most a seal is turned either way, in degrees.
const MOST_TILT: f32 = 7.0;

/// The seals for payments made at `paid` (Unix seconds), oldest first, the seasons as they fall on
/// that side of the equator.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn support_stamps(paid: Vec<i64>, southern: bool) -> Vec<Stamp> {
    paid.into_iter()
        .map(|paid_s| {
            let (month, day) = nori_look::theme::utc_date(paid_s);
            // An even spread from the time alone (Knuth's multiplicative hash).
            let spread = (paid_s as u64).wrapping_mul(2_654_435_761) % 1_000;
            let tilt = (spread as f32 / 999.0 * 2.0 - 1.0) * MOST_TILT;
            Stamp { paid_s, season: nori_look::theme::season_now(month, day, southern).season, tilt }
        })
        .collect()
}

/// How the payment is taken: Stripe's client secret for the sheet or form in the app, or the address of
/// the checkout page.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum Payment {
    InApp { client_secret: String },
    InBrowser { url: String },
}

/// A payment started: how to take it, and the settings with the login the service now knows this app
/// by (None when it already had one).
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct SupportStart {
    pub payment: Payment,
    pub change: Option<SettingChange>,
}

/// A season as the support service names it.
fn season_word(season: Season) -> &'static str {
    match season {
        Season::Spring => "spring",
        Season::Summer => "summer",
        Season::Autumn => "autumn",
        Season::Winter => "winter",
    }
}

#[derive(Deserialize)]
struct Started {
    login: String,
    secret: Option<String>,
    url: Option<String>,
}

#[derive(Deserialize)]
struct Book {
    paid: Vec<i64>,
    monthly: bool,
}

#[derive(Deserialize)]
struct Link {
    url: String,
}

/// The supporter as the service last told it (`paid` oldest first), `login` kept.
fn told(was: &Supporter, mut paid: Vec<i64>, monthly: bool) -> Supporter {
    paid.sort_unstable();
    Supporter { login: was.login.clone(), paid, monthly }
}

#[cfg_attr(feature = "ffi", uniffi::export)]
impl Client {
    /// Starts paying `amount` (minor units) in `currency`, once or monthly, taken `pay_in` the app or the
    /// browser. The first payment makes the login the service knows this app by. The season it is stamped
    /// in (on that side of the equator, `southern`) is the one the browser's page shows once it is paid.
    pub async fn support_start(&self, currency: String, amount: u64, cadence: Cadence, pay_in: PayIn, southern: bool) -> NetResult<SupportStart> {
        let login = self.settings().prefs(|p| p.support.login.clone());
        let body = serde_json::json!({
            "currency": currency.to_ascii_lowercase(),
            "amount": amount,
            "monthly": cadence == Cadence::Monthly,
            "login": (!login.is_empty()).then_some(&login),
            "in": match pay_in { PayIn::App => "app", PayIn::Browser => "browser" },
            "season": season_word(support_stamps(vec![nori_time::unix_ms() / 1000], southern)[0].season),
        });
        let started: Started = self.support_call("start", body).await?;
        let payment = match (started.secret, started.url) {
            (Some(client_secret), _) => Payment::InApp { client_secret },
            (None, Some(url)) => Payment::InBrowser { url },
            (None, None) => return Err(NetError::Parse { reason: "support start: no secret or url".into() }),
        };
        let change = (started.login != login).then(|| self.settings().change(|p| crate::settings::StoredPrefs { support: Supporter { login: started.login, ..p.support.clone() }, ..p.clone() })).flatten();
        Ok(SupportStart { payment, change })
    }

    /// Asks the service what this app has paid, and keeps it: the changed settings, None when nothing
    /// changed or the app has never paid.
    pub async fn support_refresh(&self) -> NetResult<Option<SettingChange>> {
        let was = self.settings().prefs(|p| p.support.clone());
        if was.login.is_empty() {
            return Ok(None);
        }
        let book: Book = self.support_call("stamps", serde_json::json!({ "login": was.login })).await?;
        let now = told(&was, book.paid, book.monthly);
        Ok(self.settings().change(|p| crate::settings::StoredPrefs { support: now, ..p.clone() }))
    }

    /// The page where a monthly payment is changed or stopped.
    pub async fn support_manage(&self) -> NetResult<String> {
        let login = self.settings().prefs(|p| p.support.login.clone());
        let link: Link = self.support_call("manage", serde_json::json!({ "login": login })).await?;
        Ok(link.url)
    }
}

impl Client {
    async fn support_call<T: serde::de::DeserializeOwned>(&self, path: &str, body: serde_json::Value) -> NetResult<T> {
        let base = SERVICE_URL.filter(|u| !u.is_empty()).ok_or_else(|| NetError::Parse { reason: "this build has no support service".into() })?;
        let request = Exchange { url: format!("{}/{path}", base.trim_end_matches('/')), json: Some(body.to_string()), timeout_ms: TIMEOUT_MS, ..Default::default() };
        let r = self.transport.send(request).await?;
        if !(200..300).contains(&r.status) {
            return Err(NetError::Http { status: r.status });
        }
        serde_json::from_slice(&r.body).map_err(|e| NetError::Parse { reason: format!("support {path}: {e}") })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_amounts() {
        let cases = [
            ("PLN", "40", TypedAmount::Fine { minor: 4_000 }),
            ("PLN", "12,5", TypedAmount::Fine { minor: 1_250 }),
            ("PLN", "12.05", TypedAmount::Fine { minor: 1_205 }),
            ("EUR", "1 000", TypedAmount::Fine { minor: 100_000 }),
            ("JPY", "1500", TypedAmount::Fine { minor: 1_500 }),
            ("JPY", "1500.5", TypedAmount::Unreadable),
            ("PLN", "12.345", TypedAmount::Unreadable),
            ("PLN", "", TypedAmount::Unreadable),
            ("PLN", "1,000.00", TypedAmount::Unreadable),
            ("PLN", "-5", TypedAmount::Unreadable),
            ("PLN", "4", TypedAmount::TooLittle { least: 500 }),
            ("EUR", "5000.01", TypedAmount::TooMuch { most: 500_000 }),
            ("XYZ", "10", TypedAmount::Unreadable),
        ];
        for (currency, typed, want) in cases {
            assert_eq!(support_typed(currency.into(), typed.into()), want, "{typed:?} in {currency}");
        }
    }

    #[test]
    fn a_choice_comes_to_its_amount() {
        let other = |typed: &str| support_choice("PLN".into(), Cadence::Once, SupportPick::Other, typed.into());
        let cases = [
            (support_first_choice("pl".into()), Some(4_000)),
            (support_choice("PLN".into(), Cadence::Monthly, SupportPick::Offered { index: 0 }, "12".into()), Some(1_000)),
            (support_choice("PLN".into(), Cadence::Once, SupportPick::Offered { index: 9 }, String::new()), None),
            (other("12,50"), Some(1_250)),
            (other("4"), None),
            (other(""), None),
        ];
        for (choice, want) in cases {
            assert_eq!(choice.amount, want, "{choice:?}");
        }
        assert_eq!(other("4").typed, Some(TypedAmount::TooLittle { least: 500 }));
        assert_eq!(support_choice("XYZ".into(), Cadence::Once, SupportPick::Offered { index: 0 }, String::new()).currency.code, "EUR");
    }

    #[test]
    fn offered_amounts_are_payable() {
        for c in support_currencies() {
            for a in c.once.iter().chain(&c.monthly) {
                assert!((c.least..=c.most).contains(a), "{} {a}", c.code);
            }
            assert!(c.once.windows(2).all(|w| w[0] < w[1]) && c.monthly.windows(2).all(|w| w[0] < w[1]), "{} rises", c.code);
        }
    }

    #[test]
    fn currency_by_country() {
        for (region, want) in [("pl", "PLN"), ("DE", "EUR"), ("US", "USD"), ("BR", "USD"), ("LI", "CHF")] {
            assert_eq!(support_currency_for(region.into()), want, "{region}");
            assert!(support_currencies().iter().any(|c| c.code == want));
        }
    }

    #[test]
    fn stamps_take_their_season_and_a_steady_tilt() {
        // 2026-04-10, 2026-07-20, 2026-10-15, 2027-01-05 (UTC noon).
        let paid = vec![1_775_822_400, 1_784_548_800, 1_792_065_600, 1_799_150_400];
        let north: Vec<Season> = support_stamps(paid.clone(), false).iter().map(|s| s.season).collect();
        assert_eq!(north, [Season::Spring, Season::Summer, Season::Autumn, Season::Winter]);
        let south: Vec<Season> = support_stamps(paid.clone(), true).iter().map(|s| s.season).collect();
        assert_eq!(south, [Season::Autumn, Season::Winter, Season::Spring, Season::Summer]);
        let tilts: Vec<f32> = support_stamps(paid.clone(), false).iter().map(|s| s.tilt).collect();
        assert_eq!(tilts, support_stamps(paid, false).iter().map(|s| s.tilt).collect::<Vec<_>>());
        assert!(tilts.iter().all(|t| t.abs() <= MOST_TILT) && tilts.windows(2).any(|w| w[0] != w[1]));
    }
}
