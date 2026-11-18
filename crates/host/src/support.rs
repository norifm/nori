//! The Support page of the clients whose payment Stripe's checkout page takes in the browser (the desktop,
//! the terminal, the iPod): money as it is written, the country a choice starts in, and starting a payment.

use nori_core::client::{Client, NetResult};
use nori_core::support::{PayIn, Payment, SupportChoice, SupportCurrency};
use nori_core::transport::{block_on, NetError};

/// `minor` units of `c` as English writes money: "€10", "$12.50", "40 zł", "CHF 5", "1,000 Kč"; whole
/// amounts without decimals.
pub fn money(minor: u64, c: &SupportCurrency) -> String {
    let number = number(minor, c);
    match c.code.as_str() {
        "EUR" => format!("€{number}"),
        "USD" => format!("${number}"),
        "GBP" => format!("£{number}"),
        "JPY" => format!("¥{number}"),
        "CAD" => format!("CA${number}"),
        "AUD" => format!("A${number}"),
        "PLN" => format!("{number} zł"),
        "CZK" => format!("{number} Kč"),
        "SEK" | "NOK" | "DKK" => format!("{number} kr"),
        code => format!("{code} {number}"),
    }
}

/// Currency `code`'s English name: "Euro", "Polish Zloty"; the code itself for one not offered.
pub fn currency_name(code: &str) -> &str {
    match code {
        "EUR" => "Euro",
        "USD" => "US Dollar",
        "GBP" => "British Pound",
        "PLN" => "Polish Zloty",
        "CHF" => "Swiss Franc",
        "CZK" => "Czech Koruna",
        "SEK" => "Swedish Krona",
        "NOK" => "Norwegian Krone",
        "DKK" => "Danish Krone",
        "CAD" => "Canadian Dollar",
        "AUD" => "Australian Dollar",
        "JPY" => "Japanese Yen",
        other => other,
    }
}

/// `minor` units of `c` as a plain number, grouped by thousands: "40", "1,000", "12.50".
pub fn number(minor: u64, c: &SupportCurrency) -> String {
    let scale = 10u64.pow(c.digits as u32);
    let whole = (minor / scale).to_string();
    let mut grouped = String::new();
    for (i, ch) in whole.chars().enumerate() {
        if i > 0 && (whole.len() - i).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(ch);
    }
    match minor % scale {
        0 => grouped,
        part => format!("{grouped}.{part:0width$}", width = c.digits as usize),
    }
}

/// The country `lang` names, a POSIX locale such as `pl_PL.UTF-8`: "PL"; empty when it names none.
pub fn region(lang: &str) -> String {
    lang.split(['.', '@']).next().and_then(|l| l.split_once('_')).map_or_else(String::new, |(_, r)| r.to_ascii_uppercase())
}

/// The country this computer's locale names (`LC_ALL`, `LC_MONETARY`, then `LANG`); empty when none does.
pub fn region_here() -> String {
    ["LC_ALL", "LC_MONETARY", "LANG"].into_iter().filter_map(|v| std::env::var(v).ok()).map(|l| region(&l)).find(|r| !r.is_empty()).unwrap_or_default()
}

/// Starts paying what `choice` comes to on Stripe's checkout page: the page's address, for the browser.
/// None while nothing payable is chosen. `southern`: this side of the equator is, for the season stamped.
pub fn checkout(client: &Client, choice: &SupportChoice, southern: bool) -> NetResult<Option<String>> {
    let Some(amount) = choice.amount else { return Ok(None) };
    let started = block_on(client.support_start(choice.currency.code.clone(), amount, choice.cadence, PayIn::Browser, southern))?;
    match started.payment {
        Payment::InBrowser { url } => Ok(Some(url)),
        Payment::InApp { .. } => Err(NetError::Parse { reason: "support start: a payment for the app, not the browser".into() }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nori_core::support::support_currencies;

    #[test]
    fn money_as_english_writes_it() {
        let of = |code: &str| support_currencies().into_iter().find(|c| c.code == code).unwrap();
        let cases = [("EUR", 1_000, "€10"), ("USD", 1_250, "$12.50"), ("PLN", 4_000, "40 zł"), ("CZK", 100_000, "1,000 Kč"), ("CHF", 500, "CHF 5"), ("JPY", 1_500, "¥1,500"), ("SEK", 5, "0.05 kr")];
        for (code, minor, want) in cases {
            assert_eq!(money(minor, &of(code)), want, "{code} {minor}");
        }
    }

    #[test]
    fn every_currency_offered_has_a_name() {
        for c in support_currencies() {
            assert_ne!(currency_name(&c.code), c.code, "{}", c.code);
        }
    }

    #[test]
    fn region_from_a_locale() {
        for (lang, want) in [("pl_PL.UTF-8", "PL"), ("en_us", "US"), ("de_DE@euro", "DE"), ("C", ""), ("", "")] {
            assert_eq!(region(lang), want, "{lang}");
        }
    }
}
