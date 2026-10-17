<script lang="ts">
  // Starting: a new build waiting takes over; one tab holds the databases; the profile signed in last
  // opens, or sign in.
  import { ask, boot, facts, onMovedAway, playerSoon, takeOver } from "../rpc";
  import { drawn } from "../state/library.svelte";
  import { player } from "../state/player.svelte";
  import { remote } from "../state/remote.svelte";
  import { W } from "../words";
  import Shell from "./Shell.svelte";

  let phase = $state<"starting" | "login" | "elsewhere" | "in">("starting");
  let failed = $state("");
  let busy = $state(false);
  let url = $state(location.origin);
  let user = $state("admin");
  let password = $state("admin");

  /** A new build waiting in the service worker takes over now, and the page loads again: true then.
   * Otherwise the service worker is registered (or checked for a new build). */
  async function updated(): Promise<boolean> {
    if (!("serviceWorker" in navigator)) return false;
    const reg = await navigator.serviceWorker.getRegistration();
    if (reg?.waiting && navigator.serviceWorker.controller) {
      const taken = new Promise((r) => navigator.serviceWorker.addEventListener("controllerchange", r, { once: true }));
      reg.waiting.postMessage("take over");
      await taken;
      location.reload();
      return true;
    }
    void navigator.serviceWorker.register("sw.js").then(() => (facts.serviceWorker = true));
    return false;
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
    if (await ask<boolean>("resume")) enter();
    else phase = "login";
  }

  void start();
</script>

{#if phase === "in"}
  <Shell />
{:else if phase === "login"}
  <form id="login" class="gate" onsubmit={signIn}>
    <h1>{W.appName}</h1>
    <p class="sub">{W.signInLead}</p>
    <input class="field" bind:value={url} aria-label={W.server} placeholder={W.server} autocomplete="url" />
    <input class="field" bind:value={user} aria-label={W.user} placeholder={W.user} autocomplete="username" />
    <input class="field" type="password" bind:value={password} aria-label={W.password} placeholder={W.password} autocomplete="current-password" />
    <button class="pill solid" disabled={busy}>{busy ? W.signingIn : W.signIn}</button>
    {#if failed}<p class="error">{W.signInFailed(failed)}</p>{/if}
  </form>
{:else if phase === "elsewhere"}
  <div class="gate">
    <h1>{W.elsewhere}</h1>
    <p class="sub">{W.elsewhereLead}</p>
    <button class="pill solid" id="use-here" onclick={() => void takeOver()}>{W.useHere}</button>
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
    font-weight: 800;
    letter-spacing: -0.03em;
  }
  .sub {
    margin: 0 0 12px;
  }
  .pill {
    margin-top: 8px;
    min-height: 40px;
  }
</style>
