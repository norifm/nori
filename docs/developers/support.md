# Supporting nori

The Support page takes a payment to nori through Stripe and keeps a stamp book: one seal per payment that
went through, each in the accent of the season it was paid in (`nori_core::support::support_stamps`).

## Pieces

- `crates/core/src/support.rs`: the currencies and amounts offered (five per cadence, then Other), what a
  choice comes to (`support_choice`), typed amounts, the seals, and the calls to the support service. What the app knows (its login, when it paid, whether a monthly payment runs) is
  the hidden `support` setting, so a settings backup carries it.
- `support/worker`: the support service, a Cloudflare Worker over Stripe. It keeps nothing itself: an app
  is a Stripe customer, its login `<customer id>.<secret>`, the secret's SHA-256 in the customer's
  metadata.
  - `POST /start` `{currency, amount, monthly, login?, in: "app" | "browser", season}`: a PaymentIntent (once)
    or an incomplete subscription (monthly) for the app's payment sheet, or a Checkout page for the browser.
    `season` is the one the payment is stamped in, as the app counts it on its side of the equator.
  - `POST /stamps` `{login}`: every payment that went through, and whether a monthly one runs.
  - `POST /manage` `{login}`: Stripe's customer portal for a monthly payment's card and receipts.
  - `POST /stop` `{login}`: Stripe's page stopping the monthly payment (at the end of the month paid for);
    the page confirming it thanks for the months given, counted from the paid invoices. The two pages are
    two portal configurations the Worker makes the first time, known by their `nori` metadata: cancelling
    only in `stop`, so the thanks is never skipped. A monthly payment set to stop no longer counts as
    running.
  - `GET /done`: where Checkout and the portal send the browser back: paid (`?paid=1&season=…`), the seal of
    that season stamped and thanks; left (`?left=1`), the seal's empty place; back from the portal, the way
    back alone.
- `crates/host/src/support.rs`: what the clients that pay in the browser share: money and currency names
  as English writes them, the country a choice starts in, and starting a Checkout page.
- Clients, each with the page from About (Settings on the web): Android shows Stripe's PaymentSheet (the
  `paymentsheet` library, Google Pay included); the web mounts Stripe's Payment Element in a sheet
  (`web/src/pages/Support.svelte` over `crates/web/src/support.rs`; a bank's page returns to
  `#/support/paid`). The desktop, the terminal and the iPod open Checkout in the browser and ask the service
  again when they come back into focus; the terminal also shows the page's link and a QR code, the iPod a QR
  code to pay from a phone, as iOS 12's Safari may be too old for Checkout.
- A seal more than the page opened with (a payment just made, a month's) is thanked for, the newest when
  several come at once: over the whole app the page is veiled, the seal is pressed down in the middle with
  "Thank you" under it, then carried to its empty place in the book, which the next empty place then grows
  in beside. A touch ends it; under the Motion setting it is the thanks alone, still. The terminal shows it
  as a box until a key. A client asks the service only while the page is open, so the thanks never plays
  anywhere else.

## Builds

A build has a Support page only when it is given the service: `NORI_SUPPORT_URL` (the Worker) and
`NORI_STRIPE_KEY` (Stripe's publishable key) in the environment of the Rust build. Without them
`support_service()` is None and no client shows the page. A `pk_test_` key is Stripe's test mode and
Google Pay's test environment.

CI takes both from the repository variables of the same names: the nightly, the full release, the
host packages and the web image put them in every build (into Docker too, through `tools/ipod.sh` and
`tools/host-release/build.py`) and fail at their first step when either is empty, so no published
build goes out without the page. Gradle rebuilds the core when they change.

The Worker runs twice, the same code: `nori-support` over Stripe's test mode, for development, and
`nori-support-live` over live mode, the one the repository variables name. Deploying: `cd support/worker
&& npx wrangler@4 deploy` (add `--name nori-support-live` for the live one), its Stripe key set once with
`npx wrangler@4 secret put STRIPE_SECRET` (the same `--name`). The live key is a restricted one: write on
Customers, PaymentIntents, Products, Checkout Sessions, Customer portal, Prices and Subscriptions, read on
Charges and Invoices, nothing else.

Trying it: Stripe's test cards (4242 4242 4242 4242, any future date and CVC) in a test-mode build. The
test bridge sets the stamp book directly: `tools/app.sh set support '{"login":"","paid":[1775822400],"monthly":false}'`.
