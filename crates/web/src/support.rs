//! The Support page's calls: the stamp book and the build's support service, what is chosen (the core's
//! `support_choice`), and paying through Stripe's Payment Element on the page.

use nori_core::support::{support_choice, support_currencies, support_first_choice, support_service, support_stamps, Cadence, PayIn, Payment, SupportChoice, SupportPage, SupportPick, TypedAmount};
use serde_json::{json, Value};
use wasm_bindgen::prelude::*;

use crate::{error, Nori};

/// What is chosen as the page reads it: the offered amounts in minor units, the pick (-1 for Other), and
/// the amount it comes to.
fn chosen(c: &SupportChoice) -> Value {
    let typed = match c.typed {
        Some(TypedAmount::TooLittle { least }) => json!({ "least": least }),
        Some(TypedAmount::TooMuch { most }) => json!({ "most": most }),
        _ => Value::Null,
    };
    json!({
        "currency": c.currency.code,
        "digits": c.currency.digits,
        "monthly": c.cadence == Cadence::Monthly,
        "offered": c.offered,
        "pick": match c.pick { SupportPick::Offered { index } => index as i64, SupportPick::Other => -1 },
        "other": c.other,
        "typed": typed,
        "amount": c.amount,
    })
}

#[wasm_bindgen]
impl Nori {
    /// Whether this build has a support service, so a Support page.
    pub fn support_offered(&self) -> bool {
        support_service().is_some()
    }

    /// The build's support service (null without one: no page), the stamp book on that side of the equator
    /// (`zone`'s), the season `month`/`day` stamps in, whether a monthly payment runs, the currencies offered
    /// and what is first chosen in country `region`.
    pub fn support(&self, month: u32, day: u32, zone: String, region: String) -> String {
        let Some(service) = support_service() else { return "null".into() };
        let southern = nori_look::theme::southern(&zone);
        let prefs = self.app.settings.current().unwrap_or_default();
        let seals: Vec<Value> = support_stamps(prefs.support.paid, southern).iter().map(|s| json!({ "season": s.season as u8, "tilt": s.tilt })).collect();
        json!({
            "key": service.publishable_key,
            "test": service.test,
            "seals": seals,
            "next": nori_look::theme::season_now(month, day, southern).season as u8,
            "running": prefs.support.monthly,
            "currencies": support_currencies().iter().map(|c| c.code.clone()).collect::<Vec<_>>(),
            "first": chosen(&support_first_choice(region)),
        })
        .to_string()
    }

    /// `pick` (-1: Other) among `currency`'s amounts, every month or once, with `other` typed for Other.
    pub fn support_choice(&self, currency: String, monthly: bool, pick: i32, other: String) -> String {
        let pick = if pick < 0 { SupportPick::Other } else { SupportPick::Offered { index: pick as u32 } };
        chosen(&support_choice(currency, if monthly { Cadence::Monthly } else { Cadence::Once }, pick, other)).to_string()
    }

    /// Starts paying `amount` (minor units) of `currency` in the Payment Element on the page: its client
    /// secret. The first payment keeps the login the service knows this browser by.
    pub async fn support_start(&self, currency: String, amount: u32, monthly: bool, zone: String) -> Result<String, JsValue> {
        let client = self.client().ok_or_else(|| error("not signed in"))?;
        let cadence = if monthly { Cadence::Monthly } else { Cadence::Once };
        let started = client.support_start(currency, amount as u64, cadence, PayIn::App, nori_look::theme::southern(&zone)).await.map_err(said)?;
        match started.payment {
            Payment::InApp { client_secret } => Ok(client_secret),
            Payment::InBrowser { .. } => Err(error("the support service answered with a checkout page")),
        }
    }

    /// Asks the support service what this browser has paid; the settings keep it.
    pub async fn support_refresh(&self) -> Result<(), JsValue> {
        let client = self.client().ok_or_else(|| error("not signed in"))?;
        client.support_refresh().await.map_err(said)?;
        Ok(())
    }

    /// Stripe's page for the monthly payment: its card and receipts, or (`stop`) stopping it.
    pub async fn support_page(&self, stop: bool) -> Result<String, JsValue> {
        let client = self.client().ok_or_else(|| error("not signed in"))?;
        client.support_page(if stop { SupportPage::Stop } else { SupportPage::Manage }).await.map_err(said)
    }
}

/// A refusal the page words: `currency` when a monthly payment runs in another, else why.
fn said(e: nori_core::transport::NetError) -> JsValue {
    match e {
        nori_core::transport::NetError::Http { status: 409 } => error("currency"),
        e => error(e),
    }
}
