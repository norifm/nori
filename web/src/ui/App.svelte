<script lang="ts">
  // Starting: a new build waiting takes over; one tab holds the databases; the profile signed in last
  // opens, or sign in. A page served by a Subsonic server signs in to that server only; one served
  // elsewhere asks for the server's address.
  import { ask, boot, facts, onMovedAway, playerSoon, takeOver } from "../rpc";
  import { drawn } from "../state/library.svelte";
  import { player } from "../state/player.svelte";
  import { refreshTheme } from "../state/theme.svelte";
  import { remote } from "../state/remote.svelte";
  import { W } from "../words";
  import Shell from "./Shell.svelte";

  let phase = $state<"starting" | "login" | "elsewhere" | "in">("starting");
  let failed = $state("");
  let busy = $state(false);
  let url = $state("");
  /** The page's own address is the server's. */
  let own = $state(false);
  let user = $state("admin");
  let password = $state("admin");

  /** Longest a start waits for a new build to arrive, ms: on a slow network the page starts as it is. */
  const UPDATE_WAIT_MS = 8000;

  /** Whether `worker`, installing or waiting, is ready to take over within the wait. */
  function installed(worker: ServiceWorker): Promise<boolean> {
    if (worker.state === "installed") return Promise.resolve(true);
    return new Promise((done) => {
      setTimeout(() => done(false), UPDATE_WAIT_MS);
      worker.addEventListener("statechange", () => {
        if (worker.state === "installed") done(true);
        if (worker.state === "redundant") done(false);
      });
    });
  }

  /** Looks for a new build on every start: one that arrives takes over and the page loads again, so
   * nothing old ever runs beside what is new. True then. */
  async function updated(): Promise<boolean> {
    if (!("serviceWorker" in navigator)) return false;
    const reg = await navigator.serviceWorker.register("sw.js", { updateViaCache: "none" });
    facts.serviceWorker = true;
    // Offline the running build goes on.
    await reg.update().catch(() => {});
    const next = reg.installing ?? reg.waiting;
    if (!next || !navigator.serviceWorker.controller || !(await installed(next))) return false;
    const taken = new Promise((r) => navigator.serviceWorker.addEventListener("controllerchange", r, { once: true }));
    next.postMessage("take over");
    await taken;
    location.reload();
    return true;
  }

  function enter() {
    phase = "in";
    // The player module once the first page shows, so Play never waits for it.
    void drawn.then(playerSoon);
  }

  async function signIn(e: SubmitEvent) {
    e.preventDefault();
    const t0 = performance.now();
    busy = true;
    try {
      await ask("login", url.replace(/\/$/, ""), user, password);
      facts.loginMs = Math.round(performance.now() - t0);
      enter();
    } catch (err) {
      failed = String((err as Error).message ?? err);
    }
    busy = false;
  }

  async function start() {
    if (await updated()) return;
    player.listen();
    remote.follow();
    onMovedAway(() => {
      facts.movedAway = true;
      phase = "elsewhere";
    });
    if (!(await boot())) {
      facts.elsewhere = true;
      phase = "elsewhere";
      return;
    }
    void refreshTheme();
    if (await ask<boolean>("resume")) enter();
    else {
      own = await isServer(location.origin);
      if (own) url = location.origin;
      phase = "login";
    }
  }

  /** Whether `at` answers as a Subsonic server: any answer of its API, signed in or not. */
  async function isServer(at: string): Promise<boolean> {
    try {
      const answer = await fetch(`${at}/rest/ping?f=json`);
      return "subsonic-response" in ((await answer.json()) as object);
    } catch {
      return false;
    }
  }

  void start();
</script>

{#if phase === "in"}
  <Shell />
{:else if phase === "login"}
  <form id="login" class="gate" onsubmit={signIn}>
    <h1>{W.appName}</h1>
    <p class="sub">{own ? W.signInOwnLead : W.signInLead}</p>
    {#if !own}<input class="field" bind:value={url} aria-label={W.server} placeholder={W.server} autocomplete="url" />{/if}
    <input class="field" bind:value={user} aria-label={W.user} placeholder={W.user} autocomplete="username" />
    <input class="field" type="password" bind:value={password} aria-label={W.password} placeholder={W.password} autocomplete="current-password" />
    <button class="pill" disabled={busy}>{busy ? W.signingIn : W.signIn}</button>
    {#if failed}<p class="error">{W.signInFailed(failed)}</p>{/if}
  </form>
{:else if phase === "elsewhere"}
  <div class="gate">
    <h1>{W.elsewhere}</h1>
    <p class="sub">{W.elsewhereLead}</p>
    <button class="pill" id="use-here" onclick={() => void takeOver()}>{W.useHere}</button>
  </div>
{:else}
  <p class="loading">{W.appName}</p>
{/if}

<style>
  .gate {
    max-width: 360px;
    margin: 18vh auto 0;
    padding: 0 var(--gutter);
    display: flex;
    flex-direction: column;
    gap: 10px;
    text-align: center;
  }
  h1 {
    font-size: 40px;
    font-weight: 700;
  }
  .sub {
    margin: 0 0 12px;
  }
  .pill {
    margin-top: 8px;
    min-height: 40px;
  }
</style>
