// The support service: starts a payment to nori with Stripe and tells an app what it has paid. An app
// is known by its login, "<Stripe customer id>.<secret>"; the customer keeps the secret's SHA-256 in
// its metadata. Stripe holds every record, so the Worker keeps no state of its own.

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

async function start(env: Env, origin: string, body: any): Promise<object> {
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
      customer: id,
      success_url: `${origin}/done?paid=1${season}`,
      cancel_url: `${origin}/done?left=1`,
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
  const paid: number[] = [];
  let after: string | undefined;
  for (;;) {
    const page = await stripe(env, "GET", "payment_intents", { customer: customer.id, limit: 100, starting_after: after, "expand[0]": "data.latest_charge" });
    for (const p of page.data) {
      if (p.status !== "succeeded" || p.latest_charge?.refunded) continue;
      paid.push(p.created);
      // The email the payment gave becomes the customer's, so Stripe's receipts reach it.
      const email = p.latest_charge?.billing_details?.email;
      if (!customer.email && email) {
        await stripe(env, "POST", `customers/${customer.id}`, { email });
        customer.email = email;
      }
    }
    if (!page.has_more) break;
    after = page.data[page.data.length - 1].id;
  }
  return { paid: paid.sort((a, b) => a - b), monthly: !!(await running(env, customer.id)) };
}

/** The customer's monthly payment that goes on next month: active and not set to stop. */
async function running(env: Env, customer: string): Promise<any | undefined> {
  const subs = await stripe(env, "GET", "subscriptions", { customer, status: "active", limit: 10 });
  return subs.data.find((s: any) => !s.cancel_at_period_end);
}

/**
 * The customer portal's two setups, found by their metadata and made the first time: `manage` changes the
 * card and shows the receipts; `stop` also cancels, and only the Stop link opens it, so the thanks for the
 * months given is on the page that confirms it. A monthly payment stops at the end of the month paid for.
 */
async function portal(env: Env, origin: string, use: "manage" | "stop"): Promise<string> {
  const all = await stripe(env, "GET", "billing_portal/configurations", { active: true, limit: 100 });
  const kept = all.data.find((c: any) => c.metadata?.nori === use);
  if (kept) return kept.id;
  const made = await stripe(env, "POST", "billing_portal/configurations", {
    "features[subscription_cancel][enabled]": use === "stop",
    "features[subscription_cancel][mode]": use === "stop" ? "at_period_end" : undefined,
    "features[payment_method_update][enabled]": true,
    "features[invoice_history][enabled]": true,
    default_return_url: `${origin}/done`,
    "metadata[nori]": use,
  });
  return made.id;
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

/** Stripe's page for the card a monthly payment uses and its receipts. */
async function manage(env: Env, origin: string, body: any): Promise<object> {
  const customer = await customerOf(env, body.login);
  const session = await stripe(env, "POST", "billing_portal/sessions", { customer: customer.id, configuration: await portal(env, origin, "manage"), return_url: `${origin}/done` });
  return { url: session.url };
}

/** Stripe's page stopping the monthly payment, which then thanks for the months given; 404 when none runs. */
async function stop(env: Env, origin: string, body: any): Promise<object> {
  const customer = await customerOf(env, body.login);
  const sub = await running(env, customer.id);
  if (!sub) throw new Failed(404, "no monthly payment");
  const session = await stripe(env, "POST", "billing_portal/sessions", {
    customer: customer.id,
    configuration: await portal(env, origin, "stop"),
    return_url: `${origin}/done`,
    "flow_data[type]": "subscription_cancel",
    "flow_data[subscription_cancel][subscription]": sub.id,
    "flow_data[after_completion][type]": "hosted_confirmation",
    "flow_data[after_completion][hosted_confirmation][custom_message]": thanksFor(await monthsPaid(env, customer.id)),
  });
  return { url: session.url };
}

const CORS = { "Access-Control-Allow-Origin": "*", "Access-Control-Allow-Methods": "POST, OPTIONS", "Access-Control-Allow-Headers": "Content-Type" };

const SEASONS = ["spring", "summer", "autumn", "winter"];
// Each season's paper, ink and accent, light and dark (crates/look/src/theme.rs).
const PAPERS: Record<string, [string, string, string, string, string, string]> = {
  spring: ["#F3F2F6", "#26262A", "#B14C66", "#101015", "#ECEBEF", "#E9A0B1"],
  summer: ["#EFF1F7", "#202229", "#4B5BB0", "#0A0C14", "#E8EAF0", "#9AA6EE"],
  autumn: ["#F6F0E6", "#241D16", "#B64D1D", "#110E0A", "#F3EDE3", "#E8743B"],
  winter: ["#F2F4F6", "#212326", "#2E5C8A", "#0B0D11", "#EBEDEF", "#7FB0E0"],
};
// The seal (android SupportScreen.kt's SEAL_BODY) and the seasons' kanji (Kanji.kt), on a 24 grid.
const BODY = "M5.5,2H18.5A3.5,3.5 0 0 1 22,5.5V18.5A3.5,3.5 0 0 1 18.5,22H5.5A3.5,3.5 0 0 1 2,18.5V5.5A3.5,3.5 0 0 1 5.5,2ZM5.85,3.75H18.15A2.1,2.1 0 0 1 20.25,5.85V18.15A2.1,2.1 0 0 1 18.15,20.25H5.85A2.1,2.1 0 0 1 3.75,18.15V5.85A2.1,2.1 0 0 1 5.85,3.75ZM5.95,4.35H18.05A1.6,1.6 0 0 1 19.65,5.95V18.05A1.6,1.6 0 0 1 18.05,19.65H5.95A1.6,1.6 0 0 1 4.35,18.05V5.95A1.6,1.6 0 0 1 5.95,4.35Z";
const FRAME = "M6.5,3H17.5A3.5,3.5 0 0 1 21,6.5V17.5A3.5,3.5 0 0 1 17.5,21H6.5A3.5,3.5 0 0 1 3,17.5V6.5A3.5,3.5 0 0 1 6.5,3Z";
const KANJI: Record<string, string> = {
  spring: "M18.51 9.39Q18.62 9.24 18.82 8.97Q19.02 8.71 19.13 8.59Q19.24 8.47 19.33 8.47Q19.46 8.47 20.09 8.94Q20.71 9.41 21.26 9.92Q21.81 10.43 21.81 10.58Q21.75 10.93 21.24 10.93H16.09Q17.35 12.05 18.92 12.87Q20.49 13.68 22.16 14.17L22.12 14.5Q21.22 14.89 20.68 15.54Q20.14 16.19 20.03 17Q18.45 16.08 17.32 14.91V17.11Q17.39 19.91 17.48 21.4Q17.32 21.65 16.53 21.94Q15.74 22.24 15.12 22.24Q14.57 22.24 14.57 21.8Q14.57 21.8 14.66 21.32V20.83H9.23V21.6Q9.23 21.71 8.96 21.91Q8.7 22.11 8.22 22.25Q7.73 22.39 7.14 22.39Q6.81 22.39 6.64 22.26Q6.48 22.13 6.48 21.95Q6.63 19.89 6.68 17.36V14.83Q4.76 16.5 2.1 17.64L1.9 17.45Q5.4 14.98 7.4 10.93H5.73Q4.3 10.96 2.69 11.24L2.12 10.12Q3.33 10.25 5.05 10.29H7.69Q8.19 9.15 8.57 7.9H7.09Q5.84 7.92 4.45 8.21L3.9 7.11Q5.11 7.24 6.83 7.28H8.77Q9.03 6.38 9.27 5.02H7.03Q5.47 5.04 3.73 5.32L3.16 4.22Q4.37 4.36 6.08 4.4H9.38Q9.58 3.43 9.58 2.33Q9.58 2 9.54 1.52L13.34 2.27Q13.28 2.84 12.53 2.84L12.24 4.4H17.21L17.85 3.52Q17.96 3.39 18.14 3.12Q18.31 2.86 18.43 2.74Q18.56 2.62 18.64 2.62Q18.78 2.62 19.37 3.07Q19.96 3.52 20.48 4.01Q21 4.51 21 4.66Q20.93 5.02 20.43 5.02H12.11Q11.8 6.31 11.47 7.28H16.66L17.26 6.47Q17.35 6.34 17.52 6.1Q17.7 5.87 17.81 5.76Q17.92 5.65 18.01 5.65Q18.14 5.65 18.69 6.07Q19.24 6.49 19.72 6.94Q20.21 7.39 20.21 7.54Q20.14 7.9 19.66 7.9H13.69Q14.38 9.22 15.43 10.29H17.87ZM14.13 10.29Q13.54 9.06 13.17 7.9H11.27Q10.83 9.17 10.22 10.29ZM9.51 13.24H14.53L15.3 12.41Q14.82 11.64 14.44 10.93H9.89Q9.32 11.92 8.57 12.87ZM14.66 13.88H9.23V16.68H14.66ZM9.23 17.31V20.22H14.66V17.31Z",
  summer: "M16.16 5.26Q16.27 5.15 16.49 4.88Q16.71 4.62 16.84 4.62Q16.97 4.62 17.54 5.03Q18.12 5.43 18.62 5.88Q19.13 6.34 19.13 6.49Q18.91 6.75 18.34 6.88V13.55Q18.31 13.66 17.96 13.84Q17.61 14.01 17.14 14.16Q16.66 14.3 16.27 14.3H16.03L18.07 16.3Q17.94 16.45 17.74 16.51Q17.54 16.56 17.1 16.61Q15.98 17.99 14.16 19.16Q17.13 19.84 21.86 19.84L21.83 20.11Q20.54 20.61 20.36 22.31Q17.65 22.06 15.66 21.56Q13.67 21.05 12.18 20.22Q8.22 22.02 2.36 22.37L2.25 22.06Q7.27 21.12 10.68 19.2Q9.65 18.39 8.68 17.14Q5.95 19.09 2.63 19.97L2.47 19.73Q4.67 18.61 6.5 16.92Q8.33 15.22 9.32 13.53H8.08V13.84Q8.08 13.97 7.76 14.17Q7.45 14.36 6.97 14.51Q6.5 14.65 6.02 14.65Q5.75 14.65 5.62 14.52Q5.49 14.39 5.49 14.21Q5.6 13.02 5.64 11.04V9.22Q5.64 6.45 5.36 4.71L8.24 5.89H10.57Q10.68 4.51 10.68 4.11H6.19Q4.63 4.14 2.89 4.42L2.32 3.32Q3.53 3.45 5.25 3.5H17.61L18.27 2.59Q18.38 2.44 18.58 2.18Q18.78 1.91 18.89 1.79Q19 1.67 19.08 1.67Q19.22 1.67 19.84 2.13Q20.47 2.59 21.02 3.1Q21.57 3.61 21.57 3.76Q21.48 4.11 20.98 4.11H13.32Q12.48 5.28 11.91 5.89H15.63ZM15.85 8.18V6.51H8.08V8.18ZM8.08 10.54H15.85V8.82H8.08ZM15.85 11.17H8.08V12.89H15.85ZM14.49 15.51 15.85 14.3V13.53H10.37Q12.42 13.97 12.42 14.21Q12.42 14.32 12.13 14.39L11.63 14.54Q11.01 15.18 10.68 15.51ZM10 16.12Q9.47 16.56 9.12 16.83Q10.24 17.77 11.85 18.46Q13.34 17.4 14.35 16.12Z",
  autumn: "M8.77 7.61Q8.85 7.48 9 7.25Q9.14 7.02 9.24 6.91Q9.34 6.8 9.43 6.8Q9.56 6.8 10.02 7.2Q10.48 7.61 10.88 8.07Q11.27 8.54 11.27 8.69Q11.19 9.04 10.72 9.04H7.8V10.82Q9.32 11.44 10.03 12.28Q10.75 13.11 10.75 13.93Q10.75 14.47 10.43 14.85Q10.11 15.22 9.67 15.22Q9.18 15.22 8.74 14.78Q8.72 13.31 7.8 11.53V16.32Q7.84 19.05 7.97 21.49Q7.78 21.76 7.18 22.04Q6.59 22.33 5.82 22.33Q5.51 22.33 5.34 22.2Q5.18 22.06 5.18 21.89Q5.33 19.67 5.38 17V14.34Q4.04 16.32 2.12 17.99L1.86 17.75Q3.02 16.08 3.85 13.8Q4.67 11.53 5.16 9.04H4.21Q3.42 9.06 2.58 9.35L2.03 8.25Q2.98 8.38 4.32 8.43H5.38V5.23Q3.31 5.54 2.17 5.61L2.1 5.35Q3.77 4.69 5.62 3.63Q7.47 2.57 8.5 1.69L11.25 4.14Q11.05 4.33 10.64 4.33Q10.31 4.33 9.87 4.18Q8.92 4.49 7.8 4.75V8.43H8.28ZM17.48 10.43Q17.98 9.46 18.42 8.23Q18.86 6.99 19.04 6.09Q21.9 7.65 21.9 7.98Q21.9 8.1 21.68 8.14L21.24 8.23Q19.28 10.38 17.26 11.77Q17.87 14.47 19.05 16.26Q20.23 18.04 22.21 19.16L22.19 19.4Q21.28 19.62 20.7 20.4Q20.12 21.18 19.92 22.35Q18.2 20.83 17.37 17.9Q16.53 14.96 16.25 10.25Q16.03 13.2 15.3 15.37Q14.57 17.53 12.93 19.29Q11.3 21.05 8.41 22.35L8.19 22Q10.68 20.28 11.96 18.13Q13.23 15.97 13.64 13.4Q14.05 10.82 14.05 7.06Q14.05 5.79 13.97 4.17Q13.89 2.55 13.76 1.78Q16.97 2.07 16.97 2.38Q16.97 2.46 16.8 2.57L16.42 2.84V3.34Q16.55 8.03 17.13 11.11Q17.19 10.98 17.28 10.81Q17.37 10.65 17.48 10.43ZM12.33 6.58Q12.81 7.96 12.81 9.28Q12.81 10.19 12.58 10.87Q12.35 11.55 11.96 11.97Q11.54 12.38 10.97 12.38Q10.61 12.38 10.31 12.21Q10 12.03 9.84 11.72Q9.73 11.44 9.73 11.2Q9.73 10.8 9.95 10.46Q10.17 10.12 10.55 9.86Q11.1 9.37 11.57 8.43Q12.04 7.48 12.04 6.55Z",
  winter: "M15.96 4.02Q16.07 3.94 16.29 3.72Q16.51 3.5 16.65 3.39Q16.8 3.28 16.88 3.28Q17.02 3.28 17.59 3.8Q18.16 4.33 18.68 4.92Q19.19 5.5 19.19 5.65Q19.08 5.85 18.88 5.92Q18.67 5.98 18.2 6.01Q16.6 8.56 14.22 10.49Q17.43 11.99 22.3 12.56V12.82Q21.46 13.02 20.92 13.64Q20.38 14.25 20.18 15.22Q15.39 14.06 12.51 11.75Q8.13 14.56 2.08 15.53L1.95 15.24Q7.36 13.46 11.05 10.36Q9.6 8.75 8.7 6.75Q6.06 9.46 3.07 10.8L2.89 10.62Q4.04 9.74 5.14 8.45Q6.24 7.15 7.16 5.63Q7.73 4.71 8.21 3.51Q8.68 2.31 8.85 1.39Q12.31 2.31 12.31 2.59Q12.31 2.66 12.13 2.71L11.71 2.79Q11.12 3.78 10.39 4.77H15.19ZM9.91 5.39Q9.51 5.89 9.1 6.34Q10.24 8.1 12.18 9.35Q14.09 7.44 15.19 5.39ZM9.18 13.99Q10.06 13.95 10.39 13.95Q12.11 13.95 13.22 14.31Q14.33 14.67 14.83 15.21Q15.32 15.75 15.32 16.32Q15.32 16.81 15 17.14Q14.68 17.47 14.2 17.47Q13.96 17.47 13.72 17.38Q13.1 16.7 11.82 15.82Q10.55 14.94 9.1 14.28ZM6.32 17.99Q7.47 17.95 7.97 17.95Q11.21 17.95 13.27 18.48Q15.32 19.01 16.22 19.77Q17.13 20.52 17.13 21.27Q17.13 21.76 16.8 22.05Q16.47 22.35 15.96 22.35Q15.74 22.35 15.48 22.29Q14.16 21.32 11.63 20.17Q9.1 19.03 6.26 18.28Z",
};

/**
 * Where Checkout and the customer portal send the browser back. Paid: the seal of the season it was stamped
 * in pressed onto the page, and thanks. Left Checkout: the seal's empty place, nothing paid. Back from the
 * portal: the way back alone. The paper is the season's, light or dark as the system is.
 */
function done(url: URL): string {
  const paid = url.searchParams.get("paid") === "1";
  const left = url.searchParams.get("left") === "1";
  const given = url.searchParams.get("season") ?? "";
  const season = SEASONS.includes(given) ? given : "autumn";
  const [page, ink, accent, darkPage, darkInk, darkAccent] = PAPERS[season];
  const kanji = `<path d="${KANJI[season]}" transform="translate(12 12) scale(.62) translate(-12 -12)"/>`;
  const seal = !paid && !left
    ? ""
    : paid
    ? `<svg class="seal stamped" viewBox="0 0 24 24" aria-hidden="true"><defs><mask id="cut" maskUnits="userSpaceOnUse" x="0" y="0" width="24" height="24"><rect width="24" height="24" fill="#fff"/><g fill="#000">${kanji}</g></mask></defs><g transform="rotate(-4 12 12)"><path d="${BODY}" fill-rule="evenodd" fill="var(--accent)" mask="url(#cut)"/></g></svg>`
    : `<svg class="seal" viewBox="0 0 24 24" aria-hidden="true"><path d="${FRAME}" fill="none" stroke="var(--quiet)" stroke-width="1.5"/><g fill="var(--quiet)" opacity=".85"><path d="${KANJI[season]}" transform="translate(12 12) scale(.56) translate(-12 -12)"/></g></svg>`;
  const words = paid ? "<h1>Thanks.</h1><p>You can go back to nori now.</p>" : left ? "<p>Nothing was paid. You can go back to nori now.</p>" : "<p>You can go back to nori now.</p>";
  return `<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>nori</title><style>
:root{color-scheme:light dark;--page:${page};--ink:${ink};--accent:${accent};--quiet:color-mix(in srgb,var(--ink) 55%,var(--page))}
@media (prefers-color-scheme:dark){:root{--page:${darkPage};--ink:${darkInk};--accent:${darkAccent}}}
body{margin:0;min-height:100vh;display:grid;place-items:center;background:var(--page);color:var(--ink);font:17px/1.5 Inter,system-ui,-apple-system,"Segoe UI",sans-serif;padding:16px;box-sizing:border-box;text-align:center}
main{display:flex;flex-direction:column;align-items:center;gap:10px}
.seal{width:96px;height:96px;margin-bottom:18px}
.stamped{animation:stamp .5s cubic-bezier(.3,1.6,.5,1)}
@keyframes stamp{from{transform:scale(1.5)}to{transform:scale(1)}}
@media (prefers-reduced-motion:reduce){.stamped{animation:none}}
h1{margin:0;font:600 24px/1.2 Inter,system-ui,sans-serif;letter-spacing:-.01em}
p{margin:0;color:var(--quiet)}
</style></head><body><main>${seal}${words}</main></body></html>`;
}

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    const url = new URL(request.url);
    if (request.method === "GET" && url.pathname === "/done") return new Response(done(url), { headers: { "Content-Type": "text/html; charset=utf-8" } });
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
        default: return json(404, { error: "not found" });
      }
    } catch (e) {
      const status = e instanceof Failed ? e.status : 500;
      console.log(`${url.pathname}: ${(e as Error).message}`);
      return json(status, { error: status === 401 ? "login" : status === 409 ? "currency" : "failed" });
    }
  },
};
