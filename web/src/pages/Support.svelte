<script lang="ts">
  // Supporting nori, as Android's page: the stamp book, a seal per payment in the accent of the season it was
  // paid in, five to a row; then what to give, which stays at the foot of the window whole while the book
  // scrolls under it: the amount (the offered ones as chips, then Other, typed), every month, the currency,
  // and Give, which opens Stripe's Payment Element in a sheet. What is chosen and what it comes to are the
  // core's (crates/web/src/support.rs); the words and the money as the browser writes it are here.
  // A method that pays on its own page (a bank's) comes back to `#/support/paid`. A seal that comes while the
  // page is open is thanked for: the page veiled, the seal stamped in the middle with the thanks under it, then
  // carried to its place in the book and pressed there.
  import { tick } from "svelte";
  import { ask } from "../rpc";
  import { route } from "../state/route.svelte";
  import { today, worn } from "../state/theme.svelte";
  import { W } from "../words";
  import Page from "../ui/Page.svelte";

  type Choice = { currency: string; digits: number; monthly: boolean; offered: number[]; pick: number; other: string; typed: { least?: number; most?: number } | null; amount: number | null };
  type Book = { key: string; test: boolean; seals: { season: number; tilt: number }[]; next: number; running: boolean; currencies: string[]; first: Choice };
  type Stripe = {
    elements: (o: object) => { create: (kind: string, o?: object) => { mount: (el: HTMLElement) => void } };
    confirmPayment: (o: object) => Promise<{ error?: { message?: string } }>;
  };

  const SEASONS = ["spring", "summer", "autumn", "winter"];
  const region = (navigator.language.split("-")[1] ?? "").toUpperCase();
  const zone = () => today()[2];

  let book = $state<Book | null>(null);
  let failed = $state<string | undefined>();
  let choice = $state<Choice | null>(null);
  /** Seals the book had when last read: one more is thanked for as it comes. */
  let seen = -1;

  async function read() {
    const [month, day] = today();
    try {
      const b = JSON.parse(await ask<string>("support", month, day, zone(), region)) as Book | null;
      if (!b) return void (failed = W.supportFailed);
      const fresh = seen >= 0 && b.seals.length > seen ? b.seals.length - 1 : -1;
      seen = b.seals.length;
      book = b;
      choice ??= b.first;
      if (fresh >= 0) void thank(fresh);
    } catch (e) {
      failed = String(e);
    }
  }
  async function refresh() {
    await ask("support_refresh").catch(() => {});
    await read();
  }
  // What the settings keep first, then what the service says: a payment made away from the page (a bank's,
  // a month's) is thanked for as it lands.
  $effect(() => {
    void route.visit;
    void read().then(refresh);
  });
  // Back in this tab from a bank's page or another one: what was paid there.
  $effect(() => {
    const seen = () => document.visibilityState === "visible" && void refresh();
    document.addEventListener("visibilitychange", seen);
    return () => document.removeEventListener("visibilitychange", seen);
  });

  async function choose(currency: string, monthly: boolean, pick: number, other: string) {
    choice = JSON.parse(await ask<string>("support_choice", currency, monthly, pick, other)) as Choice;
  }
  const pick = (i: number) => choice && void choose(choice.currency, choice.monthly, i, choice.other);
  const type = (text: string) => choice && void choose(choice.currency, choice.monthly, -1, text);

  /** `minor` units of the choice's currency as the browser writes money: "€10", "40 zł". */
  function money(minor: number, c: Choice) {
    const scale = 10 ** c.digits;
    const whole = minor % scale === 0;
    return new Intl.NumberFormat(undefined, { style: "currency", currency: c.currency, minimumFractionDigits: whole ? 0 : c.digits, maximumFractionDigits: whole ? 0 : c.digits }).format(minor / scale);
  }
  const plain = (minor: number, c: Choice) => new Intl.NumberFormat(undefined, { maximumFractionDigits: c.digits }).format(minor / 10 ** c.digits);
  const limit = $derived(choice?.typed?.least != null ? W.supportLeast(money(choice.typed.least, choice)) : choice?.typed?.most != null ? W.supportMost(money(choice.typed.most, choice)) : "");

  // Paying: Stripe's script, its Payment Element in a sheet, and why it didn't go through.
  let stripe: Stripe | null = null;
  let paying = $state(false);
  let starting = $state(false);
  let sheet = $state<HTMLDialogElement>();
  let mount = $state<HTMLDivElement>();
  let elements: ReturnType<Stripe["elements"]> | null = null;
  let said = $state("");

  function stripeJs(key: string): Promise<Stripe> {
    const w = window as unknown as { Stripe?: (key: string) => Stripe };
    if (w.Stripe) return Promise.resolve(w.Stripe(key));
    return new Promise((done, fail) => {
      const s = document.createElement("script");
      s.src = "https://js.stripe.com/v3/";
      s.onload = () => done(w.Stripe!(key));
      s.onerror = () => fail(new Error(W.supportFailed));
      document.head.append(s);
    });
  }

  /** Stripe's form in the page's own paper, ink, accent and corners. */
  function appearance() {
    const css = getComputedStyle(document.documentElement);
    const v = (name: string) => css.getPropertyValue(name).trim();
    return {
      theme: "flat",
      variables: { colorPrimary: v("--accent"), colorBackground: v("--surface"), colorText: v("--ink"), colorTextSecondary: v("--ink2"), fontFamily: "Inter, system-ui, sans-serif", borderRadius: "12px" },
      rules: { ".Input": { border: `1px solid ${v("--hair")}` }, ".Tab": { border: `1px solid ${v("--hair")}` } },
    };
  }

  async function give() {
    if (!book || !choice?.amount || starting) return;
    starting = true;
    said = "";
    try {
      stripe ??= await stripeJs(book.key);
      const secret = await ask<string>("support_start", choice.currency, choice.amount, choice.monthly, zone());
      elements = stripe.elements({ clientSecret: secret, appearance: appearance() });
      paying = true;
    } catch (e) {
      said = String(e).includes("currency") ? W.supportOtherCurrency : W.supportFailed;
    } finally {
      starting = false;
    }
  }
  // The sheet opens once it is drawn, the form in it.
  $effect(() => {
    if (!paying || !sheet || !mount || !elements) return;
    elements.create("payment", { layout: "tabs" }).mount(mount);
    sheet.showModal();
  });
  async function pay() {
    if (!stripe || !elements) return;
    const back = `${location.href.split("#")[0]}#/support/paid`;
    const { error } = await stripe.confirmPayment({ elements, confirmParams: { return_url: back }, redirect: "if_required" });
    if (error) return void (said = error.message ?? W.supportFailed);
    sheet?.close();
    await refresh();
  }
  // The thanks: which seal, and the one in the book left empty until it is carried there.
  let thanks = $state<{ season: number; tilt: number; index: number } | null>(null);
  let veiled = $state<HTMLDialogElement>();
  let skipped = false;
  const wait = (ms: number) => new Promise((done) => setTimeout(done, ms));
  const run = (el: Element | null | undefined, frames: Keyframe[], o: KeyframeAnimationOptions) =>
    el && !skipped ? el.animate(frames, { fill: "both", ...o }).finished.catch(() => {}) : Promise.resolve();

  async function thank(index: number) {
    if (!book || thanks) return;
    thanks = { season: book.seals[index].season, tilt: book.seals[index].tilt, index };
    skipped = false;
    await tick();
    const d = veiled;
    if (!d) return;
    d.showModal();
    const [veil, big, words] = [".veil", ".big", ".thank"].map((c) => d.querySelector(c));
    if (document.documentElement.classList.contains("still")) {
      await wait(1800);
      return land(false);
    }
    // The stamp comes down onto the page and presses it; then the thanks.
    void run(veil, [{ opacity: 0 }, { opacity: 1 }], { duration: 320, easing: "ease-out" });
    await run(big, [{ transform: "translateY(-32px) scale(1.6)", opacity: 0 }, { opacity: 1, offset: 0.6 }, { transform: "none", opacity: 1 }], { duration: 440, delay: 120, easing: "cubic-bezier(0.55, 0, 0.9, 0.45)" });
    void run(big, [{ transform: "none" }, { transform: "scale(1.05, 0.95)", offset: 0.35 }, { transform: "none" }], { duration: 220, easing: "ease-out", composite: "add" });
    await run(words, [{ transform: "translateY(10px)", opacity: 0 }, { transform: "none", opacity: 1 }], { duration: 420, delay: 140, easing: "ease-out" });
    // Meanwhile its place in the book glides into view under the veil.
    const slot = document.querySelectorAll(".book .seal")[index] as SVGElement | undefined;
    if (!skipped) await Promise.all([wait(1300), slot && reveal(slot)]);
    if (skipped || !slot || !big) return land(false);
    // The seal carried there: lifted a little, turned to the tilt it is printed at, set down and pressed, all
    // on the carried seal, so the book's takes over at rest.
    const to = slot.getBoundingClientRect();
    const from = big.getBoundingClientRect();
    const dx = to.left + to.width / 2 - (from.left + from.width / 2);
    const dy = to.top + to.height / 2 - (from.top + from.height / 2);
    const k = to.width / from.width;
    const tilt = thanks?.tilt ?? 0;
    void run(words, [{ opacity: 1 }, { opacity: 0 }], { duration: 220, easing: "ease-in" });
    void run(veil, [{ opacity: 1 }, { opacity: 0 }], { duration: 640, delay: 180, easing: "ease-in-out" });
    const pose = (x: number, y: number, deg: number, sx: number, sy = sx) => `translate(${x}px, ${y}px) rotate(${deg}deg) scale(${sx}, ${sy})`;
    await run(
      big,
      [
        { offset: 0, transform: pose(0, 0, 0, 1), easing: "cubic-bezier(0.45, 0, 0.55, 1)" },
        { offset: 0.42, transform: pose(dx * 0.35, dy * 0.35 - 36, tilt / 2, (1 + k) / 2 + 0.06), easing: "cubic-bezier(0.45, 0, 0.25, 1)" },
        { offset: 0.8, transform: pose(dx, dy, tilt, k), easing: "cubic-bezier(0.3, 0, 0.5, 1)" },
        { offset: 0.89, transform: pose(dx, dy, tilt, k * 1.06, k * 0.93), easing: "cubic-bezier(0.4, 0, 0.3, 1)" },
        { offset: 1, transform: pose(dx, dy, tilt, k) },
      ],
      { duration: 980 },
    );
    land(!skipped);
  }
  /** Scrolls the page until `slot` stands clear of the top bar and of what to give, eased over 600 ms. */
  function reveal(slot: Element): Promise<void> {
    const main = document.getElementById("main");
    const room = document.querySelector(".foot")?.getBoundingClientRect().top ?? innerHeight;
    const at = slot.getBoundingClientRect();
    if (!main || (at.top >= 80 && at.bottom <= room - 16)) return Promise.resolve();
    const [from, by, start] = [main.scrollTop, at.top - (room - at.height) / 2, performance.now()];
    return new Promise((done) => {
      const step = (now: number) => {
        const t = Math.min(1, (now - start) / 600);
        main.scrollTop = from + by * (1 - (1 - t) ** 3);
        if (t < 1 && !skipped) requestAnimationFrame(step);
        else done();
      };
      requestAnimationFrame(step);
    });
  }
  /** The seal in the empty place it was carried to, the next empty place appearing beside it; the veil gone. */
  async function land(carried: boolean) {
    const index = thanks?.index ?? -1;
    thanks = null;
    await tick();
    if (!carried) return;
    document.querySelectorAll(".book .seal")[index + 1]?.animate([{ transform: "scale(0.6)", opacity: 0 }, { transform: "none", opacity: 1 }], { duration: 320, delay: 60, fill: "backwards", easing: "cubic-bezier(0.2, 0.9, 0.3, 1.2)" });
  }
  /** A touch or Escape ends the thanks at once. */
  function skip() {
    skipped = true;
    veiled?.getAnimations({ subtree: true }).forEach((a) => a.finish());
  }

  /** Stripe's page for the monthly payment: its card and receipts, or (`stop`) stopping it. */
  async function open(stop: boolean) {
    try {
      location.href = await ask<string>("support_page", stop);
    } catch {
      said = W.supportFailed;
    }
  }
</script>

{#snippet seal(season: number, tilt: number, kind: string)}
  {@const tile = worn.tiles[season]}
  <svg class="seal {kind}" viewBox="0 0 24 24" aria-hidden="true" style:color={tile?.accent ?? "var(--accent)"}>
    <g transform="rotate({tilt.toFixed(2)} 12 12)">
      <path fill="currentColor" fill-rule="evenodd" d="M5.5,2H18.5A3.5,3.5 0 0 1 22,5.5V18.5A3.5,3.5 0 0 1 18.5,22H5.5A3.5,3.5 0 0 1 2,18.5V5.5A3.5,3.5 0 0 1 5.5,2ZM5.85,3.75H18.15A2.1,2.1 0 0 1 20.25,5.85V18.15A2.1,2.1 0 0 1 18.15,20.25H5.85A2.1,2.1 0 0 1 3.75,18.15V5.85A2.1,2.1 0 0 1 5.85,3.75ZM5.95,4.35H18.05A1.6,1.6 0 0 1 19.65,5.95V18.05A1.6,1.6 0 0 1 18.05,19.65H5.95A1.6,1.6 0 0 1 4.35,18.05V5.95A1.6,1.6 0 0 1 5.95,4.35Z" />
      <use href="#i-kanji-{SEASONS[season]}" x="4.56" y="4.56" width="14.88" height="14.88" style:color="var(--page)" />
    </g>
  </svg>
{/snippet}

{#snippet blank(season: number)}
  <svg class="seal next" viewBox="0 0 24 24" aria-hidden="true">
    <path fill="none" stroke="currentColor" stroke-width="1.5" d="M6.5,3H17.5A3.5,3.5 0 0 1 21,6.5V17.5A3.5,3.5 0 0 1 17.5,21H6.5A3.5,3.5 0 0 1 3,17.5V6.5A3.5,3.5 0 0 1 6.5,3Z" />
    <use href="#i-kanji-{SEASONS[season]}" x="5.28" y="5.28" width="13.44" height="13.44" />
  </svg>
{/snippet}

<Page title={W.supportTitle} back narrow ready={!!book} error={failed}>
  <p class="line">{W.supportLine}</p>
  {#if book && choice}
    <div class="book">
      <!-- While the thanks plays, the seal it is for still has its empty place, which it is carried to. -->
      {#each book.seals as s, i (i)}{#if thanks?.index === i}{@render blank(s.season)}{:else}{@render seal(s.season, s.tilt, "")}{/if}{/each}
      {#if !thanks}{@render blank(book.next)}{/if}
    </div>
    <div class="foot">
      <div class="group">
        <div class="group-row amount">
          <span class="words">{W.supportAmount}</span>
          <div class="choices" role="radiogroup" aria-label={W.supportAmount}>
            {#each choice.offered as a, i (i)}
              <button class="chip" class:on={choice.pick === i} role="radio" aria-checked={choice.pick === i} onclick={() => pick(i)}>{plain(a, choice)}</button>
            {/each}
            <button class="chip" class:on={choice.pick === -1} role="radio" aria-checked={choice.pick === -1} onclick={() => pick(-1)}>{W.supportOther}</button>
          </div>
        </div>
        {#if choice.pick === -1}
          <label class="group-row other">
            <!-- svelte-ignore a11y_autofocus -->
            <input class="typed" inputmode="decimal" placeholder="0" autofocus aria-label={W.supportOther} value={choice.other} oninput={(e) => type(e.currentTarget.value)} />
            <span class="limit" class:shown={!!limit}>{limit}</span>
          </label>
        {/if}
        <label class="group-row">
          <span class="words">{W.supportEveryMonth}</span>
          <input type="checkbox" class="switch" aria-label={W.supportEveryMonth} checked={choice.monthly} onchange={(e) => choice && void choose(choice.currency, e.currentTarget.checked, choice.pick, choice.other)} />
        </label>
        <label class="group-row">
          <span class="words">{W.supportCurrency}</span>
          <select class="value" aria-label={W.supportCurrency} value={choice.currency} onchange={(e) => choice && void choose(e.currentTarget.value, choice.monthly, choice.pick, choice.other)}>
            {#each book.currencies as c (c)}<option value={c}>{W.currencyNames[c] ?? c}</option>{/each}
          </select>
        </label>
      </div>
      <button class="pill give" disabled={!choice.amount || starting} onclick={() => void give()}>
        {choice.amount ? W.supportGive(money(choice.amount, choice), choice.monthly) : W.supportChoose}
      </button>
      {#if book.running}
        <div class="monthly">
          <button class="text-action" onclick={() => void open(false)}>{W.supportManage}</button>
          <button class="text-action" onclick={() => void open(true)}>{W.supportStop}</button>
        </div>
      {/if}
      {#if said && !paying}<p class="said">{said}</p>{/if}
    </div>
  {/if}
</Page>

{#if thanks}
  {@const tile = worn.tiles[thanks.season]}
  <dialog class="thanks" bind:this={veiled} oncancel={(e) => (e.preventDefault(), skip())} onclick={skip} aria-label={W.supportThanks}>
    <div class="veil"></div>
    <div class="moment">
      <div class="stamp">
        {@render seal(thanks.season, 0, "big")}
      </div>
      <p class="thank">{W.supportThanks}</p>
    </div>
  </dialog>
{/if}

{#if paying && choice?.amount}
  <dialog class="sheet pay" bind:this={sheet} onclose={() => (paying = false)} onclick={(e) => e.target === sheet && sheet?.close()} aria-label={W.supportTitle}>
    <span class="handle"></span>
    <div class="form" bind:this={mount}></div>
    {#if said}<p class="said">{said}</p>{/if}
    <button class="pill give" onclick={() => void pay()}>{W.supportPay(money(choice.amount, choice))}</button>
  </dialog>
{/if}

<style>
  .line {
    margin: -6px 0 0;
    color: var(--ink2);
  }
  /* Five to a row on a phone; a wider window keeps the seals at most 72px and fits more in a row, so the
     book shows two rows above what to give. */
  .book {
    display: grid;
    grid-template-columns: repeat(auto-fill, min(72px, calc((100% - 56px) / 5)));
    gap: 14px;
    margin: 24px 0 0;
  }
  .seal {
    display: block;
    width: 100%;
    height: auto;
    aspect-ratio: 1;
  }
  .seal.next {
    color: var(--ink2);
  }
  /* The thanks: over everything, the page blurred under a veil of its paper. */
  .thanks {
    position: fixed;
    inset: 0;
    width: 100%;
    height: 100%;
    max-width: none;
    max-height: none;
    margin: 0;
    padding: 0;
    border: 0;
    background: none;
    overflow: visible;
    outline: none;
  }
  .thanks::backdrop {
    background: none;
  }
  .veil {
    position: absolute;
    inset: 0;
    background: color-mix(in srgb, var(--page) 70%, transparent);
    backdrop-filter: blur(18px);
    -webkit-backdrop-filter: blur(18px);
  }
  .moment {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 10px;
    padding-bottom: 6vh;
  }
  /* The seal is drawn 2 of 24 inside its box: the box gives that back under it, so the gap is to the seal. */
  .stamp {
    position: relative;
    width: min(168px, 42vw);
    margin-bottom: calc(min(168px, 42vw) / -12);
  }
  .stamp .seal {
    will-change: transform;
  }
  /* Each part comes in by its own animation; under nori's Motion setting they are simply there. */
  :global(html:not(.still)) .veil,
  :global(html:not(.still)) .stamp .seal,
  :global(html:not(.still)) .thank {
    opacity: 0;
  }
  .thank {
    margin: 0;
    font-family: var(--display);
    font-weight: 700;
    letter-spacing: -0.3px;
    font-size: 34px;
    /* Its line starts at the top of the capitals, so the gap above is to the letters. */
    line-height: 1;
    margin-top: -0.26em;
    /* A touch left of center: "Th" is heavy, "you" light, and centered it leaned right. */
    position: relative;
    left: -1.5px;
    color: var(--ink);
  }
  /* What to give stays at the window's foot over the book, which fades under it as it scrolls; the page's
     paper below it keeps the book from showing between it and the window's edge. */
  .foot {
    position: sticky;
    bottom: calc(var(--bar-h) + 8px);
    margin-top: 26px;
    padding-top: 28px;
    background: linear-gradient(to bottom, transparent, var(--page) 28px);
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .foot::after {
    content: "";
    position: absolute;
    top: 100%;
    inset-inline: 0;
    height: calc(var(--bar-h) + 8px + env(safe-area-inset-bottom));
    background: var(--page);
  }
  .amount {
    flex-direction: column;
    align-items: stretch;
    gap: 10px;
    padding-block: 12px;
  }
  .choices {
    display: grid;
    grid-template-columns: repeat(6, 1fr);
    gap: 6px;
  }
  .choices .chip {
    justify-content: center;
    padding: 0;
  }
  .other {
    min-height: 60px;
  }
  .typed {
    flex: 1;
    min-width: 0;
    border: 0;
    background: none;
    color: var(--ink);
    font: 600 28px/1 inherit;
    letter-spacing: -0.01em;
    caret-color: var(--accent);
    outline: none;
  }
  .limit {
    color: var(--ink2);
    font-size: 13px;
    opacity: 0;
    transition: opacity 0.2s;
  }
  .limit.shown {
    opacity: 1;
  }
  .value {
    border: 0;
    background: none;
    color: var(--ink2);
    font: inherit;
    font-size: 15px;
    text-align: right;
  }
  .give {
    width: 100%;
    justify-content: center;
  }
  .give:disabled {
    opacity: 0.38;
  }
  .monthly {
    display: flex;
    justify-content: center;
    flex-wrap: wrap;
    gap: 4px 20px;
  }
  .said {
    margin: 0;
    text-align: center;
    color: var(--ink2);
    font-size: 14px;
  }
  .pay {
    display: flex;
    flex-direction: column;
    gap: 16px;
    padding-inline: var(--gutter);
  }
</style>
