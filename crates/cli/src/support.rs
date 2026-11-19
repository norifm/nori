//! The Support page's state: what is chosen (the core's `support_choice`), the line selected, and the
//! checkout page given last while its seal has not come. Stripe's checkout page in the browser takes the
//! payment (`nori_host::support::checkout`); its link and QR code are shown for a terminal with no browser.

use nori_core::support::{support_choice, support_currencies, support_first_choice, Cadence, SupportChoice, SupportPage, SupportPick};

use crate::app::{Cmd, Overlay, Sel, Target};
use crate::settings_view::Opened;

/// A line of the page, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Line {
    Amount,
    EveryMonth,
    Currency,
    Give,
    /// Stripe's pages for a monthly payment, while one runs: its card and receipts, and stopping it.
    Manage,
    Stop,
}

/// The checkout page given last: its address, and how many seals there were then.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Given {
    pub url: String,
    pub seals: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Support {
    pub choice: SupportChoice,
    pub sel: Sel,
    /// Until a seal more than it saw comes.
    pub given: Option<Given>,
    /// How many seals the book had as the page opened, and since: one more is thanked for.
    seen: Option<usize>,
}

/// As it first opens on this computer: in the currency of the country its locale names.
impl Default for Support {
    fn default() -> Support {
        Support::new(&nori_host::support::region_here())
    }
}

impl Support {
    /// The page as it first opens in country `region`.
    pub fn new(region: &str) -> Support {
        Support { choice: support_first_choice(region.to_string()), sel: Sel::default(), given: None, seen: None }
    }

    /// The page's lines; Manage and Stop while a monthly payment runs.
    pub fn lines(monthly: bool) -> Vec<Line> {
        let mut lines = vec![Line::Amount, Line::EveryMonth, Line::Currency, Line::Give];
        if monthly {
            lines.extend([Line::Manage, Line::Stop]);
        }
        lines
    }

    pub fn line(&self, monthly: bool) -> Option<Line> {
        Self::lines(monthly).get(self.sel.at).copied()
    }

    /// ← →: the amount before or after (Other last), the currency before or after, every month or once.
    pub fn step(&mut self, monthly: bool, on: bool) {
        let c = &self.choice;
        match self.line(monthly) {
            Some(Line::Amount) => {
                let n = c.offered.len() as i64;
                let at = match c.pick {
                    SupportPick::Offered { index } => index as i64,
                    SupportPick::Other => n,
                };
                let at = (at + if on { 1 } else { -1 }).clamp(0, n);
                let pick = if at == n { SupportPick::Other } else { SupportPick::Offered { index: at as u32 } };
                self.choose(c.currency.code.clone(), c.cadence, pick);
            }
            Some(Line::EveryMonth) => self.toggle_monthly(),
            Some(Line::Currency) => {
                let all = support_currencies();
                let at = all.iter().position(|x| x.code == c.currency.code).unwrap_or(0) as i64;
                let to = (at + if on { 1 } else { -1 }).rem_euclid(all.len() as i64) as usize;
                self.choose(all[to].code.clone(), c.cadence, c.pick);
            }
            _ => {}
        }
    }

    /// enter: the amount for Other is asked for, the currency picked from all, the monthly switch flips,
    /// and Give and Manage go to the runner.
    pub fn open(&mut self, monthly: bool) -> Option<Opened> {
        let c = &self.choice;
        match self.line(monthly)? {
            Line::Amount => {
                self.choose(c.currency.code.clone(), c.cadence, SupportPick::Other);
                let text = self.choice.other.clone();
                Some(Opened::Overlay(Overlay::Input { title: crate::text::SUPPORT_OTHER.into(), text, secret: false, target: Target::SupportOther }))
            }
            Line::EveryMonth => {
                self.toggle_monthly();
                None
            }
            Line::Currency => {
                let all = support_currencies();
                let at = all.iter().position(|x| x.code == c.currency.code).unwrap_or(0);
                let options = all.iter().map(|x| (format!("{}  {}", x.code, nori_host::support::currency_name(&x.code)), x.code.clone())).collect();
                Some(Opened::Overlay(Overlay::Picker { title: crate::text::SUPPORT_CURRENCY.into(), options, sel: Sel { at, ..Sel::default() }, target: Target::SupportCurrency }))
            }
            Line::Give => c.amount.map(|_| Opened::Cmds(vec![Cmd::SupportGive(Box::new(c.clone()))])),
            Line::Manage => Some(Opened::Cmds(vec![Cmd::SupportPage(SupportPage::Manage)])),
            Line::Stop => Some(Opened::Cmds(vec![Cmd::SupportPage(SupportPage::Stop)])),
        }
    }

    /// What is typed for Other, which it chooses.
    pub fn typed(&mut self, text: String) {
        let c = &self.choice;
        self.choice = support_choice(c.currency.code.clone(), c.cadence, SupportPick::Other, text);
    }

    pub fn currency(&mut self, code: String) {
        self.choose(code, self.choice.cadence, self.choice.pick);
    }

    /// Checkout page `url` is open for a payment, with `seals` in the book.
    pub fn gave(&mut self, url: String, seals: usize) {
        self.given = Some(Given { url, seals });
    }

    /// The page opened on a book of `seals`.
    pub fn opened(&mut self, seals: usize) {
        self.seen = Some(seals);
    }

    /// The book has `seals` now: a payment given went through once there is one more. Whether there are
    /// more than the page saw, the newest to be thanked for.
    pub fn seals(&mut self, seals: usize) -> bool {
        if self.given.as_ref().is_some_and(|g| seals > g.seals) {
            self.given = None;
        }
        let more = self.seen.is_some_and(|n| seals > n);
        self.seen = Some(seals);
        more
    }

    fn toggle_monthly(&mut self) {
        let c = &self.choice;
        let cadence = if c.cadence == Cadence::Monthly { Cadence::Once } else { Cadence::Monthly };
        self.choose(c.currency.code.clone(), cadence, c.pick);
    }

    fn choose(&mut self, currency: String, cadence: Cadence, pick: SupportPick) {
        self.choice = support_choice(currency, cadence, pick, self.choice.other.clone());
    }
}
