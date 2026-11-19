//! The Support page, as Android's SupportScreen: the stamp book, then what to give (the core's
//! `support_choice`), paid on Stripe's checkout page in the browser (`nori_host::support::checkout`).

use nori_core::settings::StoredPrefs;
use nori_core::support::{support_choice, support_currencies, support_first_choice, support_stamps, Cadence, SupportChoice, SupportPick};
use nori_host::desktop::Today;
use nori_host::support::{currency_name, money, number};
use slint::{ModelRc, SharedString, VecModel};

use crate::{AppWindow, SupportSeal};

/// The page's state between visits: what is chosen, and the checkout page given last until its seal comes.
pub struct Support {
    pub choice: SupportChoice,
    /// The checkout page's address, and how many seals the book had when it was given.
    pub given: Option<(String, usize)>,
    /// How many seals the book had when last drawn: one more comes stamped.
    seen: Option<usize>,
}

impl Default for Support {
    fn default() -> Support {
        Support { choice: support_first_choice(nori_host::support::region_here()), given: None, seen: None }
    }
}

impl Support {
    /// The amount at `at` among those offered; -1 is Other.
    pub fn pick(&mut self, at: i32) {
        let pick = if at < 0 { SupportPick::Other } else { SupportPick::Offered { index: at as u32 } };
        self.choose(self.choice.currency.code.clone(), self.choice.cadence, pick);
    }

    pub fn typed(&mut self, text: String) {
        let c = &self.choice;
        self.choice = support_choice(c.currency.code.clone(), c.cadence, SupportPick::Other, text);
    }

    pub fn monthly(&mut self, on: bool) {
        self.choose(self.choice.currency.code.clone(), if on { Cadence::Monthly } else { Cadence::Once }, self.choice.pick);
    }

    /// The currency at `at` in the order offered.
    pub fn currency(&mut self, at: usize) {
        if let Some(c) = support_currencies().get(at) {
            self.choose(c.code.clone(), self.choice.cadence, self.choice.pick);
        }
    }

    fn choose(&mut self, currency: String, cadence: Cadence, pick: SupportPick) {
        self.choice = support_choice(currency, cadence, pick, self.choice.other.clone());
    }

    /// The page as `prefs` and the choice have it; a payment given goes on waiting until the book has more seals.
    pub fn fill(&mut self, ui: &AppWindow, prefs: &StoredPrefs, today: Today) {
        let paid = prefs.support.paid.len();
        if self.given.as_ref().is_some_and(|(_, seals)| paid > *seals) {
            self.given = None;
        }
        let stamps = support_stamps(prefs.support.paid.clone(), today.southern);
        let seals: Vec<SupportSeal> = stamps.iter().map(|s| SupportSeal { season: s.season as i32, tilt: s.tilt }).collect();
        ui.set_support_fresh(if self.seen.is_some_and(|n| paid > n) { paid as i32 - 1 } else { -1 });
        self.seen = Some(paid);
        ui.set_support_seals(ModelRc::new(VecModel::from(seals)));
        ui.set_support_next(nori_look::theme::season_now(today.month, today.day, today.southern).season as i32);
        let c = &self.choice;
        let amounts: Vec<SharedString> = c.offered.iter().map(|a| number(*a, &c.currency).into()).collect();
        ui.set_support_amounts(ModelRc::new(VecModel::from(amounts)));
        ui.set_support_pick(match c.pick {
            SupportPick::Offered { index } => index as i32,
            SupportPick::Other => -1,
        });
        ui.set_support_other(c.other.clone().into());
        ui.set_support_limit(crate::words::support_limit(c).into());
        ui.set_support_monthly(c.cadence == Cadence::Monthly);
        let all = support_currencies();
        let names: Vec<SharedString> = all.iter().map(|x| currency_name(&x.code).into()).collect();
        ui.set_support_currencies(ModelRc::new(VecModel::from(names)));
        ui.set_support_currency(all.iter().position(|x| x.code == c.currency.code).unwrap_or(0) as i32);
        ui.set_support_give_words(crate::words::support_give(c.amount.map(|a| money(a, &c.currency)).as_deref(), c.cadence == Cadence::Monthly).into());
        ui.set_support_can_give(c.amount.is_some());
        ui.set_support_running(prefs.support.monthly);
        ui.set_support_waiting(self.given.is_some());
    }
}
