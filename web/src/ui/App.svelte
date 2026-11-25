<script lang="ts">
  // Starting: a new build waiting takes over; one tab holds the databases; the profile signed in last
  // opens, or sign in. A page served by a Subsonic server signs in to that server only; one served
  // elsewhere asks for the server's address. An install with no profile yet welcomes first (the core's
  // `WelcomeFlow`), and once its first profile is in, asks its questions over the app. Pairing from the
  // welcome brings another device's profiles: the one in use opens once sync brings it.
  import { ask, boot, facts, onMovedAway, playerSoon, takeOver } from "../rpc";
  import { drawn } from "../state/library.svelte";
  import { player } from "../state/player.svelte";
  import { refreshTheme } from "../state/theme.svelte";
  import { remote } from "../state/remote.svelte";
  import { sync } from "../state/sync.svelte";
  import PairDevice from "./PairDevice.svelte";
  import { W } from "../words";
  import Setup from "./Setup.svelte";
  import Shell from "./Shell.svelte";
  import Welcome from "./Welcome.svelte";

  let phase = $state<"starting" | "login" | "elsewhere" | "in">("starting");
  let failed = $state("");
  let busy = $state(false);
  let url = $state("");
  /** The page's own address is the server's. */
  let own = $state(false);
  let user = $state("admin");
  let password = $state("admin");
  /** The install has no profile yet (`first_run`). */
  let first = $state(false);
  /** Where the first run's welcome is (`welcome`): its page, the sources picked by id, and what is due. */
  let welcome = $state.raw({ page: "picking", picked: [] as string[], then: "nothing" });
  /** The first run's questions while they show (`setup_steps`). */
  let setup = $state.raw<{ step: string; answers: string[] }[] | null>(null);

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
      if (first && (await welcomeOn({ kind: "added", source: "subsonic" })).then === "questions") setup = JSON.parse(await ask<string>("setup_steps"));
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
    sync.follow((open) => void synced(open).catch(() => {}));
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
      first = await ask<boolean>("first_run");
      if (first) await welcomeOn({ kind: "read" });
      phase = "login";
    }
  }

  /** Sync changed the saved profiles; `open`: the one in use changed, or another took its place. It opens
   * (in place of the welcome on a new browser); with none left, the next start signs in. */
  async function synced(open: boolean) {
    if (!open) return;
    if (!(await ask<boolean>("resume"))) {
      location.hash = "";
      return location.reload();
    }
    if (phase === "in") remote.switched();
    else enter();
  }

  /** Tells the welcome what happened (`{ kind, source }`) and shows where it is now. */
  async function welcomeOn(said: { kind: string; source?: string }) {
    welcome = JSON.parse(await ask<string>("welcome", JSON.stringify(said)));
    return welcome;
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
  {#if setup}<Setup steps={setup} end={() => (setup = null)} />{/if}
{:else if phase === "login" && first && welcome.page === "picking"}
  <Welcome picked={welcome.picked} toggle={(source) => void welcomeOn({ kind: "toggled", source })} next={() => void welcomeOn({ kind: "continue" })} pair={() => (sync.pairing = true)} />
{:else if phase === "login"}
  <form id="login" class="gate" onsubmit={signIn}>
    <h1>{W.subsonic}</h1>
    {#if !own}<input class="field" bind:value={url} aria-label={W.server} placeholder={W.server} autocomplete="url" />{/if}
    <input class="field" bind:value={user} aria-label={W.user} placeholder={W.user} autocomplete="username" />
    <input class="field" type="password" bind:value={password} aria-label={W.password} placeholder={W.password} autocomplete="current-password" />
    <button class="pill" disabled={busy}>{busy ? W.signingIn : W.signIn}</button>
    {#if failed}<p class="error">{W.signInFailed(failed)}</p>{/if}
    {#if first}<button type="button" class="text-action" onclick={() => void welcomeOn({ kind: "back" })}>{W.back}</button>{/if}
  </form>
{:else if phase === "elsewhere"}
  <div class="gate">
    <h1>{W.elsewhere}</h1>
    <button class="pill" id="use-here" onclick={() => void takeOver()}>{W.useHere}</button>
  </div>
{:else}
  <p class="loading">{W.appName}</p>
{/if}
{#if sync.pairing && (phase === "in" || phase === "login")}<PairDevice />{/if}

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
  form h1 {
    margin-bottom: 12px;
  }
  .pill {
    margin-top: 8px;
    min-height: 40px;
  }
</style>
