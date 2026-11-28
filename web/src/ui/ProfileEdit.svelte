<script lang="ts">
  // A saved profile's page, as Android's Edit: its server's address, user and password with Save, which
  // signs in with them and opens it (`profile_edit`), then back to the list; then where its plays go
  // (`scrobblers`): its own server first, built in, where it takes them, then its scrobbling accounts, each
  // with its switch and a menu to remove it, then a row adding one. It is renamed, and its icon picked, from
  // its row in the list.
  import { ask } from "../rpc";
  import { folder } from "../state/folder.svelte";
  import { reading } from "../state/library.svelte";
  import { remote } from "../state/remote.svelte";
  import type { Profile, Profiles, ProfileSources, Scrobbling, ScrobblerKindForm } from "../state/types";
  import { ui } from "../state/ui.svelte";
  import { W } from "../words";
  import AddProfile from "./AddProfile.svelte";
  import AddScrobbler from "./AddScrobbler.svelte";
  import Icon from "./Icon.svelte";
  import Page from "./Page.svelte";
  import ProfileMark from "./ProfileMark.svelte";

  let { id }: { id: string } = $props();
  let rev = $state(0);
  const page = reading("settings", async () => {
    void rev;
    // Fails for a profile that is not saved.
    const scrobbling = JSON.parse(await ask<string>("scrobblers", id)) as Scrobbling;
    const profile = (JSON.parse(await ask<string>("profiles")) as Profiles).list.find((p) => p.id === id)!;
    const kinds = JSON.parse(await ask<string>("scrobbler_kinds")) as ScrobblerKindForm[];
    const sources = JSON.parse(await ask<string>("profile_sources", id)) as ProfileSources;
    return { profile, scrobbling, kinds, sources };
  });

  /** The form as it stands; filled from the saved profile once it is read. */
  let form = $state({ url: "", user: "", password: "" });
  $effect(() => {
    void ask<string>("profile_form", id).then((j) => (form = JSON.parse(j)));
  });
  let busy = $state(false);
  let failed = $state("");
  const ready = $derived(!!form.url.trim() && !!form.user.trim());

  async function save(e: SubmitEvent) {
    e.preventDefault();
    if (!ready || busy) return;
    busy = true;
    failed = "";
    try {
      await ask("profile_edit", id, form.url.trim().replace(/\/$/, ""), form.user.trim(), form.password);
      remote.switched();
      history.back();
    } catch (err) {
      failed = W.signInFailed(String((err as Error).message ?? err));
    }
    busy = false;
  }

  /** Something of the profile changed: the page reads it again. */
  async function change(call: string, ...args: unknown[]) {
    await ask(call, id, ...args);
    rev++;
  }

  /** A merged source changed: refresh profiles, active session if open, and this page. */
  async function sourceChange(call: string, ...args: unknown[]) {
    await ask(call, id, ...args);
    remote.switched();
    ui.profilesRev++;
    void folder.opened();
    rev++;
  }

  const sourceLabel = (p: Profile) => p.label || (p.kind === "local" ? W.local : W.subsonic);
  const sourceDetail = (p: Profile) => (p.user || (p.kind === "local" ? W.local : p.host || W.subsonic));

  /** A scrobbling account being added. */
  let adding = $state(false);
  /** Picking a saved profile (or signing in to a new one) to merge as a source. */
  let pickingSource = $state(false);
  let addingSource = $state(false);
  let sourceDialog = $state<HTMLDialogElement>();
  $effect(() => {
    if (pickingSource) sourceDialog?.showModal();
  });
</script>

<Page title={page.data?.profile ? sourceLabel(page.data.profile) : W.subsonic} back narrow ready={!!page.data} error={page.error}>
  {#if page.data && page.data.profile.kind !== "local"}
    <form class="form" id="profile-form" onsubmit={save}>
      <div class="group">
        <label class="group-row"><span class="name">{W.server}</span><input class="plain" type="url" bind:value={form.url} autocomplete="url" data-field="url" /></label>
        <label class="group-row"><span class="name">{W.user}</span><input class="plain" bind:value={form.user} autocomplete="username" data-field="user" /></label>
        <label class="group-row"><span class="name">{W.password}</span><input class="plain" type="password" bind:value={form.password} autocomplete="current-password" data-field="password" /></label>
      </div>
      {#if failed}<p class="error" id="profile-failed">{failed}</p>{/if}
      <button class="pill" id="profile-save" disabled={busy || !ready}>{busy ? W.connecting : W.save}</button>
    </form>
  {/if}
  {#if page.data}
    {@const s = page.data.scrobbling}
    <div class="group-label">{W.scrobbling}</div>
    <div class="group" id="scrobblers">
      {#if s.itself}
        <label class="group-row" id="scrobble-itself">
          <span class="words"><span>{W.scrobblerServer}</span><span class="sub">{W.scrobblerBuiltIn}</span></span>
          <input type="checkbox" class="switch" aria-label={W.scrobblerServer} checked={s.itself.on} onchange={(e) => void change("scrobble_itself_switch", e.currentTarget.checked)} />
          <!-- In line with the accounts' switches, which have their menu after them. -->
          <span class="glyph" aria-hidden="true"></span>
        </label>
      {/if}
      {#each s.list as a (a.id)}
        <label class="group-row" data-scrobbler={a.id}>
          <span class="words"><span>{W.scrobbler(a.kind)}</span><span class="sub">{[a.user, a.host].filter(Boolean).join(" · ")}</span></span>
          <input type="checkbox" class="switch" aria-label={W.scrobbler(a.kind)} checked={a.on} onchange={(e) => void change("scrobbler_switch", a.id, e.currentTarget.checked)} />
          <button class="glyph" aria-label={W.more} title={W.more} onclick={(e) => ui.drop(e, [{ label: W.remove, run: () => change("scrobbler_remove", a.id) }])}><Icon name="more-horiz" /></button>
        </label>
      {/each}
      <button class="group-row" id="add-scrobbler-row" onclick={() => (adding = true)}>
        <Icon name="plus" />
        <span class="words"><span>{W.addScrobblerAccount}</span>{#if !s.list.length && !s.itself}<span class="sub">{W.scrobblingNote}</span>{/if}</span>
      </button>
    </div>
    {#if page.data.sources.shown}
      {@const src = page.data.sources}
      <div class="group-label">{W.sources}</div>
      <div class="group" id="profile-sources">
        {#each src.sources as p, i (p.id)}
          <div class="group-row" data-source={p.id}>
            <ProfileMark icon={p.icon} letter={p.letter} kind={p.kind} />
            <span class="words"><span>{sourceLabel(p)}</span><span class="sub">{sourceDetail(p)}</span></span>
            <button
              class="glyph"
              aria-label={W.more}
              title={W.more}
              onclick={(e) =>
                ui.drop(e, [
                  ...(i > 0 ? [{ label: W.moveUp, run: () => sourceChange("source_move", p.id, -1) }] : []),
                  ...(i < src.sources.length - 1 ? [{ label: W.moveDown, run: () => sourceChange("source_move", p.id, 1) }] : []),
                  { label: W.remove, run: () => sourceChange("source_remove", p.id) },
                ])}><Icon name="more-horiz" /></button
            >
          </div>
        {/each}
        <button class="group-row" id="add-source-row" onclick={() => (pickingSource = true)}>
          <Icon name="plus" />
          <span class="words">{W.addSource}</span>
        </button>
      </div>
      {#if pickingSource}
        <dialog class="menu card" id="add-source" bind:this={sourceDialog} onclose={() => (pickingSource = false)}>
          <div class="body">
            <h2>{W.addSource}</h2>
            <div class="group">
              {#each src.offered as o (o.id)}
                <button
                  class="group-row"
                  data-offered={o.id}
                  onclick={() => {
                    pickingSource = false;
                    void sourceChange("source_add", o.id);
                  }}
                >
                  <ProfileMark icon={o.icon} letter={o.letter} kind={o.kind} />
                  <span class="words"><span>{sourceLabel(o)}</span><span class="sub">{sourceDetail(o)}</span></span>
                </button>
              {/each}
              <button
                class="group-row"
                id="source-sign-in"
                onclick={() => {
                  pickingSource = false;
                  addingSource = true;
                }}
              >
                <Icon name="plus" />
                <span class="words">{W.sourceSignIn}</span>
              </button>
            </div>
            <div class="end"><button type="button" class="text-action" onclick={() => (pickingSource = false)}>{W.cancel}</button></div>
          </div>
        </dialog>
      {/if}
      {#if addingSource}
        <AddProfile
          forProfile={id}
          close={() => (addingSource = false)}
          added={() => {
            addingSource = false;
            remote.switched();
            ui.profilesRev++;
            void folder.opened();
            rev++;
          }}
        />
      {/if}
    {/if}
    {#if adding}<AddScrobbler profile={id} kinds={page.data.kinds} close={() => (adding = false)} added={() => ((adding = false), rev++)} />{/if}
  {/if}
</Page>

<style>
  .form {
    display: flex;
    flex-direction: column;
    gap: 12px;
    margin-top: 10px;
  }
  .group {
    margin: 0 -4px;
  }
  /* A field's name, then what it holds, typed in place. */
  .name {
    flex: none;
    width: 96px;
  }
  .plain {
    flex: 1;
    min-width: 0;
    height: 36px;
    border: 0;
    outline: none;
    background: transparent;
    color: var(--ink);
    font-size: 16px;
  }
  .group-row:focus-within .name {
    color: var(--accent);
  }
  .pill {
    align-self: stretch;
  }
  .error {
    margin: 0 14px;
  }
</style>
