// The support service: starts a payment to nori with Stripe and tells an app what it has paid, and its own
// page, where a browser supports nori and stops monthly support. An app is known by its login,
// "<Stripe customer id>.<secret>", and a browser by the same login in a cookie; the customer keeps the
// secret's SHA-256 in its metadata. Stripe holds every record, so the Worker keeps no state of its own.
// What the page offers and its colors are the core's (offered.json, written by crates/core/src/support.rs).

import offered from "./offered.json";

export interface Env {
  STRIPE_SECRET: string;
}

const STRIPE = "https://api.stripe.com/v1";
const STRIPE_VERSION = "2025-03-31.basil";
/** The product every monthly payment is a price of. */
const PRODUCT = "nori-support";

type Params = Record<string, string | number | boolean | undefined>;

class Failed extends Error {
  constructor(readonly status: number, message: string) {
    super(message);
  }
}

async function stripe(env: Env, method: "GET" | "POST" | "DELETE", path: string, params: Params = {}): Promise<any> {
  const form = new URLSearchParams();
  for (const [k, v] of Object.entries(params)) if (v !== undefined) form.append(k, String(v));
  const query = method === "GET" && [...form].length ? `?${form}` : "";
  const r = await fetch(`${STRIPE}/${path}${query}`, {
    method,
    headers: {
      Authorization: `Bearer ${env.STRIPE_SECRET}`,
      "Stripe-Version": STRIPE_VERSION,
      ...(method === "POST" ? { "Content-Type": "application/x-www-form-urlencoded" } : {}),
    },
    body: method === "POST" ? form : undefined,
  });
  const body: any = await r.json();
  if (!r.ok) throw new Failed(r.status === 404 ? 404 : 502, `stripe ${path}: ${body?.error?.message ?? r.status}`);
  return body;
}

async function sha256(text: string): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(text));
  return [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, "0")).join("");
}

function secret(): string {
  const bytes = crypto.getRandomValues(new Uint8Array(24));
  return btoa(String.fromCharCode(...bytes)).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

/** The customer a login names, when its secret is the one kept; 401 otherwise. */
async function customerOf(env: Env, login: unknown): Promise<any> {
  if (typeof login !== "string" || !/^cus_[A-Za-z0-9]+\.[A-Za-z0-9_-]+$/.test(login)) throw new Failed(401, "no login");
  const [id, key] = login.split(".");
  let customer: any;
  try {
    customer = await stripe(env, "GET", `customers/${id}`);
  } catch (e) {
    if (e instanceof Failed && e.status === 404) throw new Failed(401, "unknown login");
    throw e;
  }
  if (customer.deleted || customer.metadata?.nori_key !== (await sha256(key))) throw new Failed(401, "wrong login");
  return customer;
}

async function newLogin(env: Env): Promise<{ id: string; login: string }> {
  const key = secret();
  const c = await stripe(env, "POST", "customers", { description: "nori supporter", "metadata[nori_key]": await sha256(key) });
  return { id: c.id, login: `${c.id}.${key}` };
}

/** The product monthly payments are prices of, made the first time it is needed. */
async function product(env: Env): Promise<string> {
  try {
    return (await stripe(env, "GET", `products/${PRODUCT}`)).id;
  } catch (e) {
    if (!(e instanceof Failed && e.status === 404)) throw e;
    return (await stripe(env, "POST", "products", { id: PRODUCT, name: "Support nori", "metadata[nori]": "support" })).id;
  }
}

/** Starts a payment; `page`: from the service's own page, which Checkout comes back to. */
async function start(env: Env, origin: string, body: any, page = false): Promise<{ login: string; url?: string; secret?: string }> {
  const { currency, amount, monthly } = body;
  if (typeof currency !== "string" || !/^[a-z]{3}$/.test(currency)) throw new Failed(400, "currency");
  if (!Number.isInteger(amount) || amount <= 0) throw new Failed(400, "amount");
  if (typeof monthly !== "boolean" || (body.in !== "app" && body.in !== "browser")) throw new Failed(400, "cadence or in");
  const { id, login } = body.login == null ? await newLogin(env) : { id: (await customerOf(env, body.login)).id, login: body.login };
  const described = { description: "Support nori", "metadata[nori]": "support" };
  if (monthly) await clearMonthly(env, id, currency);
  if (body.in === "browser") {
    // The season the payment is stamped in, as the app counts it: the page it comes back to stamps that seal.
    const season = SEASONS.includes(body.season) ? `&season=${body.season}` : "";
    const line = {
      "line_items[0][quantity]": 1,
      "line_items[0][price_data][currency]": currency,
      "line_items[0][price_data][unit_amount]": amount,
      ...(monthly
        ? { "line_items[0][price_data][product]": await product(env), "line_items[0][price_data][recurring][interval]": "month" }
        : { "line_items[0][price_data][product_data][name]": "Support nori" }),
    };
    const session = await stripe(env, "POST", "checkout/sessions", {
      mode: monthly ? "subscription" : "payment",
      // As a payment in the app: nori is the seller (Managed Payments, on by default, would make Stripe it).
      "managed_payments[enabled]": false,
      customer: id,
      success_url: `${origin}/done?paid=1${season}${page ? "&on=page" : ""}`,
      cancel_url: page ? `${origin}/` : `${origin}/done?left=1`,
      ...(monthly ? {} : { submit_type: "donate", "payment_intent_data[description]": "Support nori", "payment_intent_data[metadata][nori]": "support" }),
      ...line,
    });
    return { login, url: session.url };
  }
  if (!monthly) {
    const intent = await stripe(env, "POST", "payment_intents", {
      amount, currency, customer: id, "automatic_payment_methods[enabled]": true, ...described,
    });
    return { login, secret: intent.client_secret };
  }
  const sub = await stripe(env, "POST", "subscriptions", {
    customer: id,
    "items[0][price_data][currency]": currency,
    "items[0][price_data][product]": await product(env),
    "items[0][price_data][unit_amount]": amount,
    "items[0][price_data][recurring][interval]": "month",
    payment_behavior: "default_incomplete",
    "payment_settings[save_default_payment_method]": "on_subscription",
    "expand[0]": "latest_invoice.confirmation_secret",
    "metadata[nori]": "support",
  });
  return { login, secret: sub.latest_invoice.confirmation_secret.client_secret };
}

/**
 * Before a new monthly payment: the customer's monthly payments left unpaid (a sheet closed before paying)
 * are cancelled, as Stripe holds a customer to one currency while any is open; a running one in another
 * currency is a conflict (409).
 */
async function clearMonthly(env: Env, customer: string, currency: string) {
  const subs = await stripe(env, "GET", "subscriptions", { customer, status: "all", limit: 100 });
  for (const s of subs.data) {
    if (s.status === "incomplete") await stripe(env, "DELETE", `subscriptions/${s.id}`);
    else if (["active", "past_due", "trialing", "unpaid"].includes(s.status) && s.currency !== currency) throw new Failed(409, "monthly in another currency");
  }
}

/** Every payment the customer made that went through, oldest first (Unix seconds), and whether a monthly one runs. */
async function stamps(env: Env, body: any): Promise<object> {
  const customer = await customerOf(env, body.login);
  const paid = await paidBy(env, customer);
  const { running, stopping } = await monthly(env, customer.id);
  return { paid, monthly: !!running, ends: stopping?.cancel_at ?? null };
}

/**
 * When the customer's payments that went through were made, oldest first (Unix seconds). The email a
 * payment gave becomes the customer's, so Stripe's receipts reach it from then on. Stripe sends a receipt
 * only to an address it has when the payment goes through, and an app's sheet gives one only with the
 * payment: while the customer has none, no payment had a receipt, so each is sent now, to the address it gave.
 */
async function paidBy(env: Env, customer: any): Promise<number[]> {
  const unsent = !customer.email;
  const paid: number[] = [];
  let after: string | undefined;
  for (;;) {
    const page = await stripe(env, "GET", "payment_intents", { customer: customer.id, limit: 100, starting_after: after, "expand[0]": "data.latest_charge" });
    for (const p of page.data) {
      if (p.status !== "succeeded" || p.latest_charge?.refunded) continue;
      paid.push(p.created);
      const charge = p.latest_charge;
      const email = charge?.billing_details?.email;
      if (!email) continue;
      if (!customer.email) {
        await stripe(env, "POST", `customers/${customer.id}`, { email });
        customer.email = email;
      }
      if (unsent && !charge.receipt_email) await stripe(env, "POST", `charges/${charge.id}`, { receipt_email: email });
    }
    if (!page.has_more) break;
    after = page.data[page.data.length - 1].id;
  }
  return paid.sort((a, b) => a - b);
}

/**
 * The customer's monthly payment that goes on next month (active, not set to stop), and one set to stop
 * at the end of the month paid for, which can still be kept going.
 */
async function monthly(env: Env, customer: string): Promise<{ running?: any; stopping?: any }> {
  const subs = await stripe(env, "GET", "subscriptions", { customer, status: "active", limit: 10 });
  return { running: subs.data.find((s: any) => !s.cancel_at_period_end), stopping: subs.data.find((s: any) => s.cancel_at_period_end) };
}

/**
 * The customer portal's two setups, found by their metadata and made the first time: `manage` changes the
 * card and shows the receipts; `stop` also cancels, and only the Stop link opens it, so the thanks for the
 * months given is on the page that confirms it. A monthly payment stops at the end of the month paid for.
 */
async function portal(env: Env, origin: string, use: "manage" | "stop" | "login"): Promise<any> {
  const all = await stripe(env, "GET", "billing_portal/configurations", { active: true, limit: 100 });
  const kept = all.data.find((c: any) => c.metadata?.nori === use);
  if (kept) return kept;
  const stops = use !== "manage";
  return stripe(env, "POST", "billing_portal/configurations", {
    "features[subscription_cancel][enabled]": stops,
    "features[subscription_cancel][mode]": stops ? "at_period_end" : undefined,
    "features[payment_method_update][enabled]": true,
    "features[invoice_history][enabled]": true,
    "login_page[enabled]": use === "login" ? true : undefined,
    default_return_url: `${origin}/done`,
    "metadata[nori]": use,
  });
}

/** How many monthly payments the customer made that went through. */
async function monthsPaid(env: Env, customer: string): Promise<number> {
  let months = 0;
  let after: string | undefined;
  for (;;) {
    const page = await stripe(env, "GET", "invoices", { customer, status: "paid", limit: 100, starting_after: after });
    months += page.data.filter((i: any) => i.amount_paid > 0).length;
    if (!page.has_more) return months;
    after = page.data[page.data.length - 1].id;
  }
}

/** The thanks on Stripe's page confirming a monthly payment stopped. */
function thanksFor(months: number): string {
  return months > 1 ? `Thank you for supporting nori for ${months} months.` : "Thank you for supporting nori for a month.";
}

/** Stripe's page for the card a monthly payment uses and its receipts, which goes `back` when done. */
async function manage(env: Env, origin: string, body: any, back = `${origin}/done`): Promise<{ url: string }> {
  const customer = await customerOf(env, body.login);
  const session = await stripe(env, "POST", "billing_portal/sessions", { customer: customer.id, configuration: (await portal(env, origin, "manage")).id, return_url: back });
  return { url: session.url };
}

/**
 * Stops the monthly payment at the end of the month paid for: in the app (`in: "app"`), when it ends and
 * how many months it paid for; else Stripe's page doing it, which then thanks for the months given (builds
 * from before the app stopped it itself). 404 when none runs.
 */
async function stop(env: Env, origin: string, body: any): Promise<object> {
  const customer = await customerOf(env, body.login);
  const sub = (await monthly(env, customer.id)).running;
  if (!sub) throw new Failed(404, "no monthly payment");
  if (body.in === "app") {
    const stopped = await stripe(env, "POST", `subscriptions/${sub.id}`, { cancel_at_period_end: true });
    return { ends: stopped.cancel_at, months: await monthsPaid(env, customer.id) };
  }
  const session = await stripe(env, "POST", "billing_portal/sessions", {
    customer: customer.id,
    configuration: (await portal(env, origin, "stop")).id,
    return_url: `${origin}/done`,
    "flow_data[type]": "subscription_cancel",
    "flow_data[subscription_cancel][subscription]": sub.id,
    "flow_data[after_completion][type]": "hosted_confirmation",
    "flow_data[after_completion][hosted_confirmation][custom_message]": thanksFor(await monthsPaid(env, customer.id)),
  });
  return { url: session.url };
}

/** Keeps a monthly payment set to stop going; 404 when none is. */
async function keep(env: Env, body: any): Promise<object> {
  const customer = await customerOf(env, body.login);
  const sub = (await monthly(env, customer.id)).stopping;
  if (!sub) throw new Failed(404, "no monthly payment stopping");
  await stripe(env, "POST", `subscriptions/${sub.id}`, { cancel_at_period_end: false });
  return {};
}

const CORS = { "Access-Control-Allow-Origin": "*", "Access-Control-Allow-Methods": "POST, OPTIONS", "Access-Control-Allow-Headers": "Content-Type" };

type Currency = (typeof offered.currencies)[number];
type Season = "spring" | "summer" | "autumn" | "winter";
const SEASONS: string[] = ["spring", "summer", "autumn", "winter"];

/** The season on `date` on that side of the equator. */
function seasonOn(date: Date, southern: boolean): Season {
  const starts = offered.seasons[southern ? "south" : "north"];
  const [month, day] = [date.getUTCMonth() + 1, date.getUTCDate()];
  const begun = starts.filter((s) => s.month < month || (s.month === month && s.day <= day));
  return (begun[begun.length - 1] ?? starts[starts.length - 1]).season as Season;
}

/** Who is asking, as Cloudflare places them: their country, side of the equator, time zone and language. */
type Visitor = { country: string; southern: boolean; zone?: string; locale: string };

function visitor(request: Request): Visitor {
  const cf = (request as { cf?: Record<string, unknown> }).cf ?? {};
  const asked = (request.headers.get("Accept-Language") ?? "").split(",")[0].split(";")[0].trim();
  const locale = asked && Intl.NumberFormat.supportedLocalesOf(asked).length ? asked : "en";
  return { country: String(cf.country ?? ""), southern: Number(cf.latitude) < 0, zone: typeof cf.timezone === "string" ? cf.timezone : undefined, locale };
}

/** `minor` units of `c` as the visitor's language writes money: "€10", "40 zł"; whole amounts without decimals. */
function money(minor: number, c: Currency, v: Visitor): string {
  const digits = minor % 10 ** c.digits === 0 ? 0 : c.digits;
  return new Intl.NumberFormat(v.locale, { style: "currency", currency: c.code, minimumFractionDigits: digits, maximumFractionDigits: digits }).format(minor / 10 ** c.digits);
}

const plain = (minor: number, c: Currency, v: Visitor) => new Intl.NumberFormat(v.locale, { maximumFractionDigits: c.digits }).format(minor / 10 ** c.digits);

/** Unix seconds as the visitor writes a day: "Dec 22". */
const day = (s: number, v: Visitor) => new Date(s * 1000).toLocaleDateString(v.locale, { month: "short", day: "numeric", timeZone: v.zone });

const NAMES = new Intl.DisplayNames(["en"], { type: "currency" });

/**
 * What the page has chosen, from its form or query: the currency (the visitor's country's first), every
 * month or once, the amount picked for each (an index into the amounts offered, or "other") and what is typed
 * for Other.
 */
type Choice = { currency: Currency; monthly: boolean; once: string; month: string; other: string };

function choiceFrom(q: URLSearchParams, v: Visitor): Choice {
  const code = q.get("currency") ?? (offered.countries as Record<string, string>)[v.country] ?? offered.elsewhere;
  const currency = offered.currencies.find((c) => c.code === code) ?? offered.currencies[0];
  const pick = (name: string, amounts: number[]) => {
    const p = q.get(name) ?? "";
    return p === "other" || (/^\d$/.test(p) && Number(p) < amounts.length) ? p : String(offered.first);
  };
  return { currency, monthly: q.get("every") === "month", once: pick("once", currency.once), month: pick("month", currency.monthly), other: q.get("other") ?? "" };
}

/** What a choice comes to in minor units, or why it can't be paid. */
function amountOf(c: Choice, v: Visitor): { minor: number } | { said: string } {
  const pick = c.monthly ? c.month : c.once;
  if (pick !== "other") return { minor: (c.monthly ? c.currency.monthly : c.currency.once)[Number(pick)] };
  const typed = Number(c.other) * 10 ** c.currency.digits;
  const minor = Math.round(typed);
  if (!c.other || !Number.isFinite(typed) || Math.abs(typed - minor) > 1e-6) return { said: "Choose an amount" };
  if (minor < c.currency.least) return { said: `At least ${money(c.currency.least, c.currency, v)}` };
  if (minor > c.currency.most) return { said: `At most ${money(c.currency.most, c.currency, v)}` };
  return { minor };
}

// The browser's login: HttpOnly, so no script on the page reads it; SameSite=Lax, so no other site's form
// sends it along (and each form's Origin is checked too). It lasts as long as a browser keeps one (400 days).
const COOKIE = "nori_login";
const keepLogin = (login: string) => `${COOKIE}=${login}; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age=34560000`;
const CLEAR_LOGIN = `${COOKIE}=; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age=0`;

function loginIn(request: Request): string | undefined {
  return (request.headers.get("Cookie") ?? "").match(/(?:^|;\s*)nori_login=([^;]*)/)?.[1] || undefined;
}

/** The customer a login names; undefined for one Stripe doesn't know or whose secret is wrong. */
async function known(env: Env, login: string | undefined): Promise<any> {
  if (!login) return undefined;
  return customerOf(env, login).catch((e) => {
    if (e instanceof Failed && e.status === 401) return undefined;
    throw e;
  });
}

/** What a browser that has supported nori has given: when each payment was, and its monthly payment. */
type Given = { paid: number[]; running?: any; stopping?: any; months: number };

async function given(env: Env, customer: any): Promise<Given> {
  const [paid, { running, stopping }] = await Promise.all([paidBy(env, customer), monthly(env, customer.id)]);
  return { paid, running, stopping, months: stopping ? await monthsPaid(env, customer.id) : 0 };
}

// The seal (android SupportScreen.kt's SEAL_BODY) and the seasons' kanji (Kanji.kt), on a 24 grid.
const BODY = "M5.5,2H18.5A3.5,3.5 0 0 1 22,5.5V18.5A3.5,3.5 0 0 1 18.5,22H5.5A3.5,3.5 0 0 1 2,18.5V5.5A3.5,3.5 0 0 1 5.5,2ZM5.85,3.75H18.15A2.1,2.1 0 0 1 20.25,5.85V18.15A2.1,2.1 0 0 1 18.15,20.25H5.85A2.1,2.1 0 0 1 3.75,18.15V5.85A2.1,2.1 0 0 1 5.85,3.75ZM5.95,4.35H18.05A1.6,1.6 0 0 1 19.65,5.95V18.05A1.6,1.6 0 0 1 18.05,19.65H5.95A1.6,1.6 0 0 1 4.35,18.05V5.95A1.6,1.6 0 0 1 5.95,4.35Z";
const FRAME = "M6.5,3H17.5A3.5,3.5 0 0 1 21,6.5V17.5A3.5,3.5 0 0 1 17.5,21H6.5A3.5,3.5 0 0 1 3,17.5V6.5A3.5,3.5 0 0 1 6.5,3Z";
const KANJI: Record<string, string> = {
  spring: "M18.51 9.39Q18.62 9.24 18.82 8.97Q19.02 8.71 19.13 8.59Q19.24 8.47 19.33 8.47Q19.46 8.47 20.09 8.94Q20.71 9.41 21.26 9.92Q21.81 10.43 21.81 10.58Q21.75 10.93 21.24 10.93H16.09Q17.35 12.05 18.92 12.87Q20.49 13.68 22.16 14.17L22.12 14.5Q21.22 14.89 20.68 15.54Q20.14 16.19 20.03 17Q18.45 16.08 17.32 14.91V17.11Q17.39 19.91 17.48 21.4Q17.32 21.65 16.53 21.94Q15.74 22.24 15.12 22.24Q14.57 22.24 14.57 21.8Q14.57 21.8 14.66 21.32V20.83H9.23V21.6Q9.23 21.71 8.96 21.91Q8.7 22.11 8.22 22.25Q7.73 22.39 7.14 22.39Q6.81 22.39 6.64 22.26Q6.48 22.13 6.48 21.95Q6.63 19.89 6.68 17.36V14.83Q4.76 16.5 2.1 17.64L1.9 17.45Q5.4 14.98 7.4 10.93H5.73Q4.3 10.96 2.69 11.24L2.12 10.12Q3.33 10.25 5.05 10.29H7.69Q8.19 9.15 8.57 7.9H7.09Q5.84 7.92 4.45 8.21L3.9 7.11Q5.11 7.24 6.83 7.28H8.77Q9.03 6.38 9.27 5.02H7.03Q5.47 5.04 3.73 5.32L3.16 4.22Q4.37 4.36 6.08 4.4H9.38Q9.58 3.43 9.58 2.33Q9.58 2 9.54 1.52L13.34 2.27Q13.28 2.84 12.53 2.84L12.24 4.4H17.21L17.85 3.52Q17.96 3.39 18.14 3.12Q18.31 2.86 18.43 2.74Q18.56 2.62 18.64 2.62Q18.78 2.62 19.37 3.07Q19.96 3.52 20.48 4.01Q21 4.51 21 4.66Q20.93 5.02 20.43 5.02H12.11Q11.8 6.31 11.47 7.28H16.66L17.26 6.47Q17.35 6.34 17.52 6.1Q17.7 5.87 17.81 5.76Q17.92 5.65 18.01 5.65Q18.14 5.65 18.69 6.07Q19.24 6.49 19.72 6.94Q20.21 7.39 20.21 7.54Q20.14 7.9 19.66 7.9H13.69Q14.38 9.22 15.43 10.29H17.87ZM14.13 10.29Q13.54 9.06 13.17 7.9H11.27Q10.83 9.17 10.22 10.29ZM9.51 13.24H14.53L15.3 12.41Q14.82 11.64 14.44 10.93H9.89Q9.32 11.92 8.57 12.87ZM14.66 13.88H9.23V16.68H14.66ZM9.23 17.31V20.22H14.66V17.31Z",
  summer: "M16.16 5.26Q16.27 5.15 16.49 4.88Q16.71 4.62 16.84 4.62Q16.97 4.62 17.54 5.03Q18.12 5.43 18.62 5.88Q19.13 6.34 19.13 6.49Q18.91 6.75 18.34 6.88V13.55Q18.31 13.66 17.96 13.84Q17.61 14.01 17.14 14.16Q16.66 14.3 16.27 14.3H16.03L18.07 16.3Q17.94 16.45 17.74 16.51Q17.54 16.56 17.1 16.61Q15.98 17.99 14.16 19.16Q17.13 19.84 21.86 19.84L21.83 20.11Q20.54 20.61 20.36 22.31Q17.65 22.06 15.66 21.56Q13.67 21.05 12.18 20.22Q8.22 22.02 2.36 22.37L2.25 22.06Q7.27 21.12 10.68 19.2Q9.65 18.39 8.68 17.14Q5.95 19.09 2.63 19.97L2.47 19.73Q4.67 18.61 6.5 16.92Q8.33 15.22 9.32 13.53H8.08V13.84Q8.08 13.97 7.76 14.17Q7.45 14.36 6.97 14.51Q6.5 14.65 6.02 14.65Q5.75 14.65 5.62 14.52Q5.49 14.39 5.49 14.21Q5.6 13.02 5.64 11.04V9.22Q5.64 6.45 5.36 4.71L8.24 5.89H10.57Q10.68 4.51 10.68 4.11H6.19Q4.63 4.14 2.89 4.42L2.32 3.32Q3.53 3.45 5.25 3.5H17.61L18.27 2.59Q18.38 2.44 18.58 2.18Q18.78 1.91 18.89 1.79Q19 1.67 19.08 1.67Q19.22 1.67 19.84 2.13Q20.47 2.59 21.02 3.1Q21.57 3.61 21.57 3.76Q21.48 4.11 20.98 4.11H13.32Q12.48 5.28 11.91 5.89H15.63ZM15.85 8.18V6.51H8.08V8.18ZM8.08 10.54H15.85V8.82H8.08ZM15.85 11.17H8.08V12.89H15.85ZM14.49 15.51 15.85 14.3V13.53H10.37Q12.42 13.97 12.42 14.21Q12.42 14.32 12.13 14.39L11.63 14.54Q11.01 15.18 10.68 15.51ZM10 16.12Q9.47 16.56 9.12 16.83Q10.24 17.77 11.85 18.46Q13.34 17.4 14.35 16.12Z",
  autumn: "M8.77 7.61Q8.85 7.48 9 7.25Q9.14 7.02 9.24 6.91Q9.34 6.8 9.43 6.8Q9.56 6.8 10.02 7.2Q10.48 7.61 10.88 8.07Q11.27 8.54 11.27 8.69Q11.19 9.04 10.72 9.04H7.8V10.82Q9.32 11.44 10.03 12.28Q10.75 13.11 10.75 13.93Q10.75 14.47 10.43 14.85Q10.11 15.22 9.67 15.22Q9.18 15.22 8.74 14.78Q8.72 13.31 7.8 11.53V16.32Q7.84 19.05 7.97 21.49Q7.78 21.76 7.18 22.04Q6.59 22.33 5.82 22.33Q5.51 22.33 5.34 22.2Q5.18 22.06 5.18 21.89Q5.33 19.67 5.38 17V14.34Q4.04 16.32 2.12 17.99L1.86 17.75Q3.02 16.08 3.85 13.8Q4.67 11.53 5.16 9.04H4.21Q3.42 9.06 2.58 9.35L2.03 8.25Q2.98 8.38 4.32 8.43H5.38V5.23Q3.31 5.54 2.17 5.61L2.1 5.35Q3.77 4.69 5.62 3.63Q7.47 2.57 8.5 1.69L11.25 4.14Q11.05 4.33 10.64 4.33Q10.31 4.33 9.87 4.18Q8.92 4.49 7.8 4.75V8.43H8.28ZM17.48 10.43Q17.98 9.46 18.42 8.23Q18.86 6.99 19.04 6.09Q21.9 7.65 21.9 7.98Q21.9 8.1 21.68 8.14L21.24 8.23Q19.28 10.38 17.26 11.77Q17.87 14.47 19.05 16.26Q20.23 18.04 22.21 19.16L22.19 19.4Q21.28 19.62 20.7 20.4Q20.12 21.18 19.92 22.35Q18.2 20.83 17.37 17.9Q16.53 14.96 16.25 10.25Q16.03 13.2 15.3 15.37Q14.57 17.53 12.93 19.29Q11.3 21.05 8.41 22.35L8.19 22Q10.68 20.28 11.96 18.13Q13.23 15.97 13.64 13.4Q14.05 10.82 14.05 7.06Q14.05 5.79 13.97 4.17Q13.89 2.55 13.76 1.78Q16.97 2.07 16.97 2.38Q16.97 2.46 16.8 2.57L16.42 2.84V3.34Q16.55 8.03 17.13 11.11Q17.19 10.98 17.28 10.81Q17.37 10.65 17.48 10.43ZM12.33 6.58Q12.81 7.96 12.81 9.28Q12.81 10.19 12.58 10.87Q12.35 11.55 11.96 11.97Q11.54 12.38 10.97 12.38Q10.61 12.38 10.31 12.21Q10 12.03 9.84 11.72Q9.73 11.44 9.73 11.2Q9.73 10.8 9.95 10.46Q10.17 10.12 10.55 9.86Q11.1 9.37 11.57 8.43Q12.04 7.48 12.04 6.55Z",
  winter: "M15.96 4.02Q16.07 3.94 16.29 3.72Q16.51 3.5 16.65 3.39Q16.8 3.28 16.88 3.28Q17.02 3.28 17.59 3.8Q18.16 4.33 18.68 4.92Q19.19 5.5 19.19 5.65Q19.08 5.85 18.88 5.92Q18.67 5.98 18.2 6.01Q16.6 8.56 14.22 10.49Q17.43 11.99 22.3 12.56V12.82Q21.46 13.02 20.92 13.64Q20.38 14.25 20.18 15.22Q15.39 14.06 12.51 11.75Q8.13 14.56 2.08 15.53L1.95 15.24Q7.36 13.46 11.05 10.36Q9.6 8.75 8.7 6.75Q6.06 9.46 3.07 10.8L2.89 10.62Q4.04 9.74 5.14 8.45Q6.24 7.15 7.16 5.63Q7.73 4.71 8.21 3.51Q8.68 2.31 8.85 1.39Q12.31 2.31 12.31 2.59Q12.31 2.66 12.13 2.71L11.71 2.79Q11.12 3.78 10.39 4.77H15.19ZM9.91 5.39Q9.51 5.89 9.1 6.34Q10.24 8.1 12.18 9.35Q14.09 7.44 15.19 5.39ZM9.18 13.99Q10.06 13.95 10.39 13.95Q12.11 13.95 13.22 14.31Q14.33 14.67 14.83 15.21Q15.32 15.75 15.32 16.32Q15.32 16.81 15 17.14Q14.68 17.47 14.2 17.47Q13.96 17.47 13.72 17.38Q13.1 16.7 11.82 15.82Q10.55 14.94 9.1 14.28ZM6.32 17.99Q7.47 17.95 7.97 17.95Q11.21 17.95 13.27 18.48Q15.32 19.01 16.22 19.77Q17.13 20.52 17.13 21.27Q17.13 21.76 16.8 22.05Q16.47 22.35 15.96 22.35Q15.74 22.35 15.48 22.29Q14.16 21.32 11.63 20.17Q9.1 19.03 6.26 18.28Z",
};

/** A seal in the accent of the season it was paid in, the kanji cut out of it. */
const seal = (season: string, cls = "seal") =>
  `<svg class="${cls}" viewBox="0 0 24 24" aria-hidden="true"><path d="${BODY}" fill-rule="evenodd" fill="var(--${season})"/><path d="${KANJI[season]}" fill="var(--page)" transform="translate(12 12) scale(.62) translate(-12 -12)"/></svg>`;

/** The next seal's empty place: its outline and kanji, quiet. */
const blank = (season: string) =>
  `<svg class="seal" viewBox="0 0 24 24" aria-hidden="true"><path d="${FRAME}" fill="none" stroke="var(--ink2)" stroke-width="1.5"/><path d="${KANJI[season]}" fill="var(--ink2)" transform="translate(12 12) scale(.56) translate(-12 -12)"/></svg>`;

// The apps' icons (iconoir), as the web client's sprite has them.
const ICONS: Record<string, string> = {
  calendar: '<path d="M15 4V2M15 4V6M15 4H10.5M3 10V19C3 20.1046 3.89543 21 5 21H19C20.1046 21 21 20.1046 21 19V10H3Z"/><path d="M3 10V6C3 4.89543 3.89543 4 5 4H7"/><path d="M7 2V6"/><path d="M21 10V6C21 4.89543 20.1046 4 19 4H18.5"/>',
  card: '<path d="M22 9V17C22 18.1046 21.1046 19 20 19H4C2.89543 19 2 18.1046 2 17V7C2 5.89543 2.89543 5 4 5H20C21.1046 5 22 5.89543 22 7V9ZM22 9H6"/>',
  stop: '<path d="M19.1414 5C17.3265 3.14864 14.7974 2 12 2C6.47715 2 2 6.47715 2 12C2 14.7255 3.09032 17.1962 4.85857 19M19.1414 5C20.9097 6.80375 22 9.27455 22 12C22 17.5228 17.5228 22 12 22C9.20261 22 6.67349 20.8514 4.85857 19M19.1414 5L4.85857 19"/>',
  keep: '<path d="M19.91 13.2C19.33 17.05 16.01 20 12 20C7.582 20 4 16.42 4 12C4 7.582 7.582 4 12 4C15.28 4 18.1 5.975 19.33 8.8"/><path d="M16 8.8H19.52C19.79 8.8 20 8.585 20 8.32V4.8"/>',
  arrow: '<path d="M9 6L15 12L9 18"/>',
};
const icon = (name: string) =>
  `<svg class="icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${ICONS[name]}</svg>`;

const escape = (text: string) => text.replace(/[&<>"']/g, (c) => `&#${c.charCodeAt(0)};`);

/**
 * A page on `season`'s paper, light or dark as the system is, in the apps' roles (page, surface, ink, its
 * quiet share, the hairline, the accent, on) and their parts: grouped rows, chips, pills, the switch.
 */
function paper(season: Season, title: string, css: string, body: string): string {
  const colors = offered.colors as Record<string, Record<"light" | "dark", Record<string, string>>>;
  const roles = (dark: boolean) => {
    const mode = dark ? "dark" : "light";
    const own = Object.entries(colors[season][mode]).map(([k, v]) => `--${k}:${v}`);
    return [...own, ...SEASONS.map((s) => `--${s}:${colors[s][mode].accent}`)].join(";");
  };
  return `<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>${title}</title>
<link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
<link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=Inter:wght@400;500;600&family=Literata:opsz,wght@7..72,700&display=swap">
<style>
:root{color-scheme:light dark;${roles(false)};--line:color-mix(in srgb,var(--ink) 10%,transparent);--veil:color-mix(in srgb,var(--ink) 6%,transparent);--raised:color-mix(in srgb,var(--ink) 5%,var(--surface));--off:color-mix(in srgb,var(--ink) 16%,var(--page));--font:Inter,system-ui,-apple-system,"Segoe UI",Roboto,sans-serif;--display:Literata,Georgia,serif}
@media (prefers-color-scheme:dark){:root{${roles(true)}}}
*{box-sizing:border-box}
body{margin:0;min-height:100vh;background:var(--page);color:var(--ink);font:15px/20px var(--font);-webkit-font-smoothing:antialiased;-webkit-tap-highlight-color:transparent}
a{color:inherit;text-decoration:none}
button,input,select{font:inherit;color:inherit}
button{background:none;border:0;padding:0;cursor:pointer}
:focus-visible{outline:2px solid var(--accent);outline-offset:2px;border-radius:6px}
h1{margin:0;font:700 31px/40px var(--display);letter-spacing:-.3px}
p{margin:0}
.icon{width:20px;height:20px;flex:none;display:block}
.group{background:var(--surface);border:.5px solid var(--hair);border-radius:18px;overflow:hidden}
.group-row{position:relative;display:flex;align-items:center;gap:14px;width:100%;min-height:52px;padding:8px 14px;font-size:16px;line-height:21px;text-align:left}
.group-row::after{content:"";position:absolute;left:14px;right:14px;bottom:0;height:1px;background:var(--line)}
.group-row:has(>.icon:first-child)::after{left:48px}
.group-row:last-child::after{display:none}
a.group-row:hover{background:color-mix(in srgb,var(--ink) 5%,transparent)}
.words{flex:1;min-width:0;display:flex;flex-direction:column}
.value,.sub{font-size:13.5px;color:var(--ink2);white-space:nowrap}
.sub{font-size:13px;line-height:17px;white-space:normal}
.group-row>.icon:last-child:not(:first-child){color:var(--ink2);margin-left:-10px}
.pill{display:inline-flex;align-items:center;justify-content:center;gap:7px;width:100%;min-height:50px;padding:0 16px;border-radius:999px;background:var(--ink);color:var(--page);font-size:15px;font-weight:600;white-space:nowrap}
.pill.quiet{background:var(--veil);color:var(--ink)}
.pill .icon{width:18px;height:18px}
${css}
</style></head><body>${body}</body></html>`;
}

/**
 * The support page, as the apps' Support page: the stamp book, one seal for each payment this browser made
 * and the next one's empty place; while a monthly payment runs or is set to stop, its row and Stop (Spring's
 * rose) or Keep going; then what to give (the amount, every month, the currency) and Give, which opens
 * Checkout. A payment made in an app is reached by Stripe's sign-in by email (the `login` portal). It
 * works without scripts: the form shows the amounts of the cadence chosen, and a changed currency
 * reloads it.
 */
async function home(env: Env, origin: string, v: Visitor, choice: Choice, customer: any, said = ""): Promise<string> {
  const [mine, signIn] = await Promise.all([
    customer ? given(env, customer) : undefined,
    portal(env, origin, "login").then((c) => c.login_page?.url as string | undefined, () => undefined),
  ]);
  const now = seasonOn(new Date(), v.southern);
  const book = (mine?.paid ?? []).map((s) => seal(seasonOn(new Date(s * 1000), v.southern))).join("") + blank(now);
  const c = choice.currency;
  const runs = mine?.running?.items?.data?.[0]?.price;
  const monthlyRow = mine?.running
    ? `<div class="group"><div class="group-row">${icon("calendar")}<span class="words">Monthly support</span><span class="value">${escape(money(runs.unit_amount, offered.currencies.find((o) => o.code === runs.currency.toUpperCase()) ?? c, v))}</span></div></div>
<form method="post" action="/monthly/card"><button class="pill quiet">${icon("card")}Card and receipts</button></form>
<form method="post" action="/monthly/stop"><button class="pill quiet stop">${icon("stop")}Stop</button></form>`
    : mine?.stopping
    ? `<div class="group"><div class="group-row">${icon("calendar")}<span class="words">Monthly support</span><span class="value">Ends ${escape(day(mine.stopping.cancel_at, v))}</span></div></div>
${mine.months ? `<p class="thanked">${mine.months === 1 ? "Thank you for a month of support." : `Thank you for ${mine.months} months of support.`}</p>` : ""}
<form method="post" action="/monthly/card"><button class="pill quiet">${icon("card")}Card and receipts</button></form>
<form method="post" action="/monthly/keep"><button class="pill quiet">${icon("keep")}Keep going</button></form>`
    : "";
  const chips = (name: "once" | "month", amounts: number[], picked: string) =>
    `<div class="choices ${name}" role="radiogroup" aria-label="Amount">${amounts
      .map((a, i) => `<input type="radio" name="${name}" value="${i}" id="${name}-${i}"${picked === String(i) ? " checked" : ""}><label class="chip" for="${name}-${i}">${escape(plain(a, c, v))}</label>`)
      .join("")}<input type="radio" name="${name}" value="other" id="${name}-other"${picked === "other" ? " checked" : ""}><label class="chip" for="${name}-other">Other</label></div>`;
  // Give names the amount picked; for Other, the amount typed is only known once sent.
  const gives = (name: "once" | "month", amounts: number[], suffix: string) =>
    amounts.map((a, i) => `<span class="says ${name}-${i}">Give ${escape(money(a, c, v))}${suffix}</span>`).join("") + `<span class="says ${name}-other">Give</span>`;
  const cadence = { once: ".give:not(:has(.switch:checked))", month: ".give:has(.switch:checked)" };
  const shows = (["once", "month"] as const)
    .flatMap((name) => [...(name === "once" ? c.once : c.monthly).map((_, i) => String(i)), "other"].map((i) => `${cadence[name]}:has(#${name}-${i}:checked) .says.${name}-${i}`))
    .join(",");
  const currencies = offered.currencies.map((o) => `<option value="${o.code}"${o.code === c.code ? " selected" : ""}>${escape(NAMES.of(o.code) ?? o.code)}</option>`).join("");
  const step = c.digits ? `0.${"0".repeat(c.digits - 1)}1` : "1";
  return paper(now, "Support nori", `
main{max-width:560px;margin:0 auto;padding:24px 20px 40px;display:flex;flex-direction:column}
.line{margin-top:2px;color:var(--ink2)}
.book{display:grid;grid-template-columns:repeat(auto-fill,min(72px,calc((100% - 56px)/5)));gap:14px;margin:24px 0 26px}
.seal{display:block;width:100%;height:auto;aspect-ratio:1}
.foot,.give{display:flex;flex-direction:column;gap:12px}
.foot{margin-bottom:12px}
.pill.stop{color:var(--spring);background:color-mix(in srgb,var(--spring) 12%,transparent)}
.thanked{margin:0 4px}
.amount{flex-direction:column;align-items:stretch;gap:10px;padding-block:12px}
.choices{display:grid;grid-template-columns:repeat(6,1fr);gap:6px}
.choices input,.give>input{position:absolute;opacity:0;pointer-events:none}
.chip{display:flex;align-items:center;justify-content:center;height:34px;border-radius:999px;border:.5px solid transparent;color:var(--ink2);font-size:14px;font-weight:500;white-space:nowrap;cursor:pointer}
.choices input:checked+.chip{background:var(--raised);border-color:var(--hair);color:var(--ink)}
.choices input:focus-visible+.chip{outline:2px solid var(--accent);outline-offset:2px}
.give:has(.switch:checked) .choices.once,.give:not(:has(.switch:checked)) .choices.month{display:none}
.other{display:none;min-height:60px}
${cadence.once}:has(#once-other:checked) .other,${cadence.month}:has(#month-other:checked) .other{display:flex}
.typed{flex:1;min-width:0;border:0;background:none;color:var(--ink);font:600 28px/1 var(--font);letter-spacing:-.01em;caret-color:var(--accent);outline:none;-moz-appearance:textfield}
.typed::-webkit-inner-spin-button,.typed::-webkit-outer-spin-button{-webkit-appearance:none;margin:0}
.limit{color:var(--ink2);font-size:13px}
.switch{appearance:none;width:51px;height:31px;border-radius:16px;background:var(--off);position:relative;cursor:pointer;flex:none;margin:0;transition:background 180ms}
.switch::after{content:"";position:absolute;top:2px;left:2px;width:27px;height:27px;border-radius:50%;background:#fff;transition:transform 180ms}
.switch:checked{background:var(--on)}
.switch:checked::after{transform:translateX(20px)}
select.value{border:0;background:none;font-size:15px;text-align:right;cursor:pointer}
.says{display:none}
${shows}{display:inline}
.said{text-align:center;color:var(--ink2);font-size:14px}
.away{margin-top:26px}
@media (min-width:720px){h1,.line{text-align:center}}
@media (prefers-reduced-motion:reduce){.switch,.switch::after{transition:none}}`, `<main>
<h1>Support nori</h1>
<p class="line">nori is and always will be free.</p>
<div class="book">${book}</div>
${monthlyRow ? `<div class="foot">${monthlyRow}</div>` : ""}
<form class="give" method="post" action="/give">
<div class="group">
<div class="group-row amount"><span>Amount</span>${chips("once", c.once, choice.once)}${chips("month", c.monthly, choice.month)}</div>
<label class="group-row other"><input class="typed" type="number" name="other" min="0" step="${step}" inputmode="decimal" placeholder="0" aria-label="Other" value="${escape(choice.other)}">${said.startsWith("At ") ? `<span class="limit">${escape(said)}</span>` : ""}</label>
<label class="group-row"><span class="words">Every month</span><input type="checkbox" class="switch" name="every" value="month"${choice.monthly ? " checked" : ""}></label>
<label class="group-row"><span class="words">Currency</span><select class="value" name="currency" onchange="location.search=new URLSearchParams(new FormData(this.form))">${currencies}</select></label>
</div>
<noscript><button class="pill quiet" formmethod="get" formaction="/">Change currency</button></noscript>
<button class="pill">${gives("once", c.once, "")}${gives("month", c.monthly, " a month")}</button>
${said && !said.startsWith("At ") ? `<p class="said">${escape(said)}</p>` : ""}
</form>
${signIn ? `<div class="group away"><a class="group-row" href="${escape(signIn)}"><span class="words"><span>Sign in with your email</span><span class="sub">Paid in the app</span></span>${icon("arrow")}</a></div>` : ""}
</main>`);
}

/**
 * Where Checkout and the customer portal send the browser back. Paid: the seal of the season it was stamped
 * in pressed onto the page, and thanks; from the support page, the way back to it. Left Checkout: the seal's
 * empty place, nothing paid. Back from the portal: the way back to nori.
 */
function done(url: URL): string {
  const paid = url.searchParams.get("paid") === "1";
  const left = url.searchParams.get("left") === "1";
  const page = url.searchParams.get("on") === "page";
  const given = url.searchParams.get("season") ?? "";
  const season = (SEASONS.includes(given) ? given : "autumn") as Season;
  const mark = paid ? seal(season, "seal stamped") : left ? blank(season) : "";
  const words = paid
    ? `<h1>Thank you</h1>${page ? `<a class="pill back" href="/">Support nori</a>` : "<p>Go back to nori.</p>"}`
    : left ? "<p>Nothing was paid.</p>" : "<p>Go back to nori.</p>";
  return paper(season, "nori", `
body{display:grid;place-items:center;text-align:center;padding:16px}
main{display:flex;flex-direction:column;align-items:center;gap:10px}
p{color:var(--ink2)}
.seal{width:96px;height:96px;margin-bottom:18px}
.back{width:auto;margin-top:14px;padding:0 24px}
.stamped{animation:stamp .5s cubic-bezier(.3,1.6,.5,1)}
@keyframes stamp{from{transform:scale(1.5)}to{transform:scale(1)}}
@media (prefers-reduced-motion:reduce){.stamped{animation:none}}`, `<main>${mark}${words}</main>`);
}

/** The page's forms: what they ask, answered with the page again or a way on (303). */
async function form(env: Env, request: Request, url: URL): Promise<Response> {
  const html = (status: number, text: string, cookie?: string) =>
    new Response(text, { status, headers: { "Content-Type": "text/html; charset=utf-8", ...(cookie ? { "Set-Cookie": cookie } : {}) } });
  const to = (location: string, cookie?: string) => new Response(null, { status: 303, headers: { Location: location, ...(cookie ? { "Set-Cookie": cookie } : {}) } });
  const v = visitor(request);
  const had = loginIn(request);
  const customer = await known(env, had);
  const forgotten = had && !customer ? CLEAR_LOGIN : undefined;
  if (request.method === "GET") return html(200, await home(env, url.origin, v, choiceFrom(url.searchParams, v), customer), forgotten);
  if (request.headers.get("Origin") !== url.origin) return new Response("forbidden", { status: 403 });
  const asked = new URLSearchParams(await request.text());
  const login = customer ? had : undefined;
  if (url.pathname === "/give") {
    const choice = choiceFrom(asked, v);
    const amount = amountOf(choice, v);
    if ("said" in amount) return html(400, await home(env, url.origin, v, choice, customer, amount.said), forgotten);
    const season = seasonOn(new Date(), v.southern);
    try {
      const started = await start(env, url.origin, { currency: choice.currency.code.toLowerCase(), amount: amount.minor, monthly: choice.monthly, login: login ?? null, in: "browser", season }, true);
      return to(started.url!, keepLogin(started.login));
    } catch (e) {
      if (!(e instanceof Failed)) throw e;
      console.log(`/give: ${e.message}`);
      const said = e.status === 409 ? "Your monthly support is in another currency" : "The payment didn't go through";
      return html(e.status, await home(env, url.origin, v, choice, customer, said), forgotten);
    }
  }
  if (!login) return to("/", forgotten);
  try {
    if (url.pathname === "/monthly/stop") await stop(env, url.origin, { login, in: "app" });
    else if (url.pathname === "/monthly/keep") await keep(env, { login });
    else if (url.pathname === "/monthly/card") return to((await manage(env, url.origin, { login }, `${url.origin}/`)).url);
    else return new Response("not found", { status: 404 });
  } catch (e) {
    // No monthly payment to stop or keep (stopped elsewhere meanwhile): the page shows what there is.
    if (!(e instanceof Failed && e.status === 404)) throw e;
  }
  return to("/");
}

const PAGE = ["/", "/give", "/monthly/stop", "/monthly/keep", "/monthly/card"];

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    const url = new URL(request.url);
    const html = (text: string, status = 200) => new Response(text, { status, headers: { "Content-Type": "text/html; charset=utf-8" } });
    if (request.method === "GET" && url.pathname === "/done") return html(done(url));
    if ((request.method === "GET" && url.pathname === "/") || (request.method === "POST" && PAGE.includes(url.pathname) && url.pathname !== "/")) {
      try {
        return await form(env, request, url);
      } catch (e) {
        console.log(`${url.pathname}: ${(e as Error).message}`);
        return html(paper(seasonOn(new Date(), visitor(request).southern), "Support nori", "body{display:grid;place-items:center;padding:16px}p{color:var(--ink2)}", "<p>Stripe isn't answering. Try again later.</p>"), 502);
      }
    }
    if (request.method === "OPTIONS") return new Response(null, { headers: CORS });
    if (request.method !== "POST") return new Response("not found", { status: 404 });
    const json = (status: number, body: object) => new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json", ...CORS } });
    try {
      const body = await request.json().catch(() => {
        throw new Failed(400, "not json");
      });
      switch (url.pathname) {
        case "/start": return json(200, await start(env, url.origin, body));
        case "/stamps": return json(200, await stamps(env, body));
        case "/manage": return json(200, await manage(env, url.origin, body));
        case "/stop": return json(200, await stop(env, url.origin, body));
        case "/keep": return json(200, await keep(env, body));
        default: return json(404, { error: "not found" });
      }
    } catch (e) {
      const status = e instanceof Failed ? e.status : 500;
      console.log(`${url.pathname}: ${(e as Error).message}`);
      return json(status, { error: status === 401 ? "login" : status === 409 ? "currency" : "failed" });
    }
  },
};
