<script lang="ts">
  // Settings. On a wide screen as the desktop's: the servers first, the pages the core lays out for a
  // browser (`settings_pages`), then the plugins, as tabs under the title, the one open below them. On a phone as
  // Android's: the root lists them, each with its glyph and what it holds, and a page (`#/settings/<page>`)
  // opens on its own. A page has its sections as small-caps labels over grouped cards, each row drawn by
  // its kind and worded here. A row says only what is not obvious from its name. Appearance is the look
  // the top bar's menu has too. Servers lists the saved profiles as Android does: the one in use checked,
  // another opened with a click, each with its icon picked from its circle and a menu to rename it, edit it
  // (its page, `#/settings/Servers/<id>`, with its scrobbling) or remove it; then adding one. Plugins come
  // before About, the last, as Android lists them; About draws itself, with its licenses page under it.
  import { ask } from "../rpc";
  import { reading } from "../state/library.svelte";
  import { folder } from "../state/folder.svelte";
  import { remote } from "../state/remote.svelte";
  import { route, href } from "../state/route.svelte";
  import { refreshTheme, worn } from "../state/theme.svelte";
  import { ui } from "../state/ui.svelte";
  import type { MusicFolderSwitch, Profile, ProfileIcon, Profiles, SettingRow, SettingsSection } from "../state/types";
  import { W } from "../words";
  import About from "../ui/About.svelte";
  import AddProfile from "../ui/AddProfile.svelte";
  import Icon from "../ui/Icon.svelte";
  import Licences from "../ui/Licences.svelte";
  import Look from "../ui/Look.svelte";
  import Page from "../ui/Page.svelte";
  import Plugins from "../ui/Plugins.svelte";
  import ProfileEdit from "../ui/ProfileEdit.svelte";
  import ProfileIconPicker from "../ui/ProfileIconPicker.svelte";
  import ProfileMark from "../ui/ProfileMark.svelte";

  const glyphs: Record<string, string> = { Playing: "play", Sound: "control-slider", Look: "palette", Lyrics: "quote-message", Library: "album-list", Data: "cloud-download", Servers: "server", Plugins: "view-grid", About: "info-circle" };
  let rev = $state(0);
  const page = reading("settings", async () => {
    void rev;
    void ui.profilesRev;
    const names = JSON.parse(await ask<string>("settings_pages")) as string[];
    const name = route.arg;
    const sections = names.includes(name) ? (JSON.parse(await ask<string>("settings_page", name)) as SettingsSection[]) : [];
    const profiles = JSON.parse(await ask<string>("profiles")) as Profiles;
    const folders = name === "Servers" || (!name && ui.wide) ? (JSON.parse(await ask<string>("music_folders").catch(() => "[]")) as MusicFolderSwitch[]) : [];
    // A build with a support service offers its page last.
    const supported = await ask<boolean>("support_offered");
    const version = name === "About" ? (JSON.parse(await ask<string>("about")) as { version: string }).version : "";
    return { names, sections, profiles, folders, supported, version };
  });
  /** The page open: on a wide screen the first while none is asked for. */
  const name = $derived(route.arg || (ui.wide ? "Servers" : ""));
  /** The core's pages but About, which follows Plugins. */
  const pages = $derived((page.data?.names ?? []).filter((n) => n !== "About"));
  const about = $derived(page.data?.names.includes("About") ?? false);
  const tabs = $derived([["Servers", W.servers], ...pages.map((n) => [n, W.settingsPages[n]?.[0] ?? n]), ["Plugins", W.plugins], ...(about ? [["About", W.settingsPages.About[0]]] : [])]);
  /** The page's sections the rows draw: Appearance is the look, drawn whole. */
  const drawn = $derived(name === "Look" ? [] : (page.data?.sections ?? []));

  async function set(setting: string, value: string) {
    await ask("setting", setting, value);
    if (setting === "season" || setting === "theme" || setting === "motion") await refreshTheme();
    rev++;
  }

  const profiles = $derived(page.data?.profiles);
  const named = (p: Profile) => p.label || W.subsonic;
  const inUse = $derived(profiles?.list.find((p) => p.on));
  /** The profile whose icon is being picked. */
  let picking = $state.raw<Profile | null>(null);
  async function pickIcon(p: Profile, icon: ProfileIcon) {
    await ask("profile_icon", p.id, icon);
    ui.profilesRev++;
  }
  /** A profile being added. */
  let adding = $state(false);

  /** The library and the player are another profile's now. */
  function opened() {
    remote.switched();
    ui.profilesRev++;
    void folder.opened();
  }
  async function open(p: Profile) {
    await ask("profile_open", p.id);
    opened();
  }
  async function remove(p: Profile) {
    if (!(await ui.confirm(W.removeProfileTitle(named(p)), W.removeProfileLine, W.remove))) return;
    // None left: the next start signs in.
    if (!(await ask<boolean>("profile_remove", p.id))) return signedOut();
    if (p.on) opened();
    else ui.profilesRev++;
  }
  const menu = (e: MouseEvent, p: Profile) =>
    ui.drop(e, [
      {
        label: W.rename,
        run: async () => {
          const name = await ui.ask(W.rename, named(p));
          if (name === null) return;
          await ask("profile_rename", p.id, name);
          ui.profilesRev++;
        },
      },
      { label: W.edit, run: () => (location.hash = `${href("settings", "Servers")}/${encodeURIComponent(p.id)}`) },
      { label: W.remove, run: () => remove(p) },
    ]);

  async function signOut() {
    await ask("logout");
    signedOut();
  }
  function signedOut() {
    location.hash = "";
    location.reload();
  }

  /** A chore that frees space: done, the page reads itself again. */
  async function clearLyrics() {
    await ask("clear_lyrics");
    ui.say(W.clearedLyrics);
    rev++;
  }

  const shownRows = (s: SettingsSection) => s.rows.filter((r) => r.shown);
  const label = (setting: string) => W.settingNames[setting] ?? setting;
</script>

{#if name === "Plugins" && (route.sub || !ui.wide)}
  <Plugins />
{:else if name === "Servers" && route.sub}
  <ProfileEdit id={route.sub} />
{:else if name === "About" && route.sub}
  <Licences />
{:else if ui.wide}
  <!-- One block centered in the page, in the top bar's column: the title, the categories as a rail at its
       left on a wide window (the settings' column ending at its right) and as chips under the title
       otherwise, over the column in its middle. A category's words and the title share one line, its pill
       drawn round them; the rail's first row stands level with the column's first header. -->
  <Page title={W.settings} narrow ready={!!page.data} error={page.error}>
    <div class="framed">
      <nav class="tabs" aria-label={W.settings}>
        {#each tabs as [n, label] (n)}
          <a class="tab" class:on={n === name} aria-current={n === name ? "page" : undefined} href={href("settings", n)} data-page={n}>{label}</a>
        {/each}
      </nav>
      <div class="column">
        {#if name === "Servers"}{@render serversPage()}{:else if name === "Plugins"}<Plugins tabbed />{:else}{@render settingsPage()}{/if}
      </div>
    </div>
  </Page>
{:else if !name}
  <Page title={W.settings} narrow ready={!!page.data} error={page.error}>
    <div class="group root">
      <a class="group-row" href={href("settings", "Servers")}><Icon name="server" /><span class="words"><span>{W.servers}</span>{#if inUse}<span class="sub">{named(inUse)}</span>{/if}</span><Icon name="nav-arrow-right" /></a>
      {#each pages as n (n)}{@render rootRow(n)}{/each}
      <a class="group-row" href={href("settings", "Plugins")}><Icon name="view-grid" /><span class="words">{W.plugins}</span><Icon name="nav-arrow-right" /></a>
      {#if about}{@render rootRow("About")}{/if}
      {#if page.data?.supported}<a class="group-row" href={href("support")}><Icon name="seal" /><span class="words">{W.supportTitle}</span><Icon name="nav-arrow-right" /></a>{/if}
    </div>
  </Page>
{:else if name === "Servers"}
  <Page title={W.servers} back narrow ready={!!page.data} error={page.error}>{@render serversPage()}</Page>
{:else}
  <Page title={W.settingsPages[name]?.[0] ?? name} back narrow ready={!!page.data} error={page.error}>{@render settingsPage()}</Page>
{/if}

{#snippet rootRow(n: string)}
  <a class="group-row" href={href("settings", n)} data-page={n}><Icon name={glyphs[n] ?? "settings"} /><span class="words"><span>{W.settingsPages[n]?.[0] ?? n}</span><span class="sub">{W.settingsPages[n]?.[1] ?? ""}</span></span><Icon name="nav-arrow-right" /></a>
{/snippet}

{#snippet serversPage()}
  <div class="group-label">{W.profiles}</div>
  <div class="group" id="profiles">
    {#each profiles?.list ?? [] as p (p.id)}
      <div class="group-row profile" class:on={p.on} data-profile={p.id}>
        {#if profiles?.picks}
          <button class="mark" aria-label={W.profileIcon} title={W.profileIcon} onclick={() => (picking = p)}><ProfileMark icon={p.icon} letter={p.letter} kind={p.kind} /></button>
        {:else}
          <ProfileMark icon={p.icon} letter={p.letter} kind={p.kind} />
        {/if}
        <button class="opens" disabled={p.on} onclick={() => void open(p)}><span class="words"><span>{named(p)}</span><span class="sub">{p.user}</span></span></button>
        {#if p.on}<span class="in-use" title={W.inUse} aria-label={W.inUse}><Icon name="check" /></span>{/if}
        {#if profiles?.picks}<button class="glyph" aria-label={W.more} title={W.more} onclick={(e) => menu(e, p)}><Icon name="more-horiz" /></button>{/if}
      </div>
    {/each}
    {#if profiles?.picks}<button class="group-row" id="add-profile-row" onclick={() => (adding = true)}><Icon name="plus" /><span class="words">{W.addProfile}</span></button>{/if}
    <button class="group-row" onclick={() => void signOut()}><Icon name="log-out" /><span class="words">{W.signOut}</span></button>
  </div>
  {#if page.data?.folders.length}
    <div class="group-label">{W.thisProfile}</div>
    <div class="group" id="music-folders">
      {#each page.data.folders as f (f.id)}
        <label class="group-row" data-folder={f.id}>
          <span class="words">{f.name}</span>
          <input
            type="checkbox"
            class="switch"
            aria-label={f.name}
            checked={f.on}
            onchange={(e) => {
              e.currentTarget.checked = f.on;
              void set("musicFolders", f.tapped);
            }}
          />
        </label>
      {/each}
    </div>
  {/if}
  {#if adding}<AddProfile close={() => (adding = false)} added={() => ((adding = false), opened())} />{/if}
  {#if picking && profiles}{@const p = picking}<ProfileIconPicker profile={p} icons={profiles.icons} pick={(icon) => void pickIcon(p, icon)} close={() => (picking = null)} />{/if}
{/snippet}

{#snippet settingsPage()}
  {#if name === "Look"}<Look />{/if}
  {#if name === "About" && page.data}<About version={page.data.version} />{/if}
  {#each drawn as sec (sec.id)}
    {@const rest = shownRows(sec)}
    {#if rest.length}
      <div class="group-label">{W.sections[sec.id] ?? sec.id}</div>
      <div class="group">
        {#each rest as r, i (i)}{@render row(r)}{/each}
      </div>
    {/if}
  {/each}
{/snippet}

{#snippet row(r: SettingRow)}
  {#if r.item.kind === "note"}
    <p class="note">{W.notes[r.item.note] ?? ""}</p>
  {:else if r.item.kind === "stored"}
    <!-- What is kept, by part: one bar in the parts' colors, then a line each, as the desktop's. -->
    {@const it = r.item}
    <div class="group-row stored" data-row="stored">
      <span class="words"><span>{W.storedHere}</span><span class="sub">{it.total}</span></span>
      <span class="bar">{#each it.parts as p, i (p.part)}<i style:width="{p.share * 100}%" style:background={worn.usage[i] ?? "var(--ink2)"}></i>{/each}</span>
      <span class="parts">{#each it.parts as p, i (p.part)}<span class="part"><i style:background={worn.usage[i] ?? "var(--ink2)"}></i>{W.storedParts[p.part] ?? p.part} · {p.size}</span>{/each}</span>
    </div>
  {:else if r.item.kind === "unsent"}
    <div class="group-row" data-row="unsent"><span class="words"><span>{W.unsent}</span><span class="sub">{W.unsentLine(r.item.waiting, r.item.dropped)}</span></span></div>
  {:else if r.item.kind === "action"}
    <div class="group-row" class:off={!r.enabled} data-action={r.item.action}>
      <span class="words"><span>{W.clearLyrics}</span><span class="sub">{W.clearLyricsLine(r.item.size)}</span></span>
      <button class="pill quiet small" disabled={!r.enabled} onclick={() => void clearLyrics()}>{W.clear}</button>
    </div>
  {:else if r.item.kind === "link"}
    <a class="group-row" href={href(r.item.to)} data-link={r.item.to}><span class="words">{W.equalizer}</span><Icon name="nav-arrow-right" /></a>
  {:else}
    {@const it = r.item}
    {#if it.type === "Switch"}
      <label class="group-row" class:under={r.depth > 0} class:off={!r.enabled} data-setting={it.name}>
        <span class="words">{label(it.name)}</span>
        <input type="checkbox" class="switch" aria-label={label(it.name)} checked={it.value === "true"} disabled={!r.enabled} onchange={(e) => void set(it.name, e.currentTarget.checked ? "true" : "false")} />
      </label>
    {:else if it.type === "Choice" && it.chips}
      <div class="group-row chips-row" class:under={r.depth > 0} class:off={!r.enabled} data-setting={it.name}>
        <span class="words">{label(it.name)}</span>
        <div class="choices" role="radiogroup" aria-label={label(it.name)}>
          {#each it.options as o (o)}
            <button class="chip" class:on={o === it.value} role="radio" aria-checked={o === it.value} disabled={!r.enabled} onclick={() => o !== it.value && void set(it.name, o)}>{W.choice(it.name, o)}</button>
          {/each}
        </div>
      </div>
    {:else if it.type === "Choice"}
      <button
        class="group-row"
        class:under={r.depth > 0}
        class:off={!r.enabled}
        data-setting={it.name}
        disabled={!r.enabled}
        onclick={(e) => ui.drop(e, it.options.map((o) => ({ label: W.choice(it.name, o), chosen: o === it.value || Number(o) === Number(it.value), run: () => set(it.name, o) })))}
        ><span class="words">{label(it.name)}</span><span class="value">{W.choice(it.name, it.value)}</span><Icon name="nav-arrow-right" /></button
      >
    {:else if it.type === "Level"}
      <div class="group-row level" class:under={r.depth > 0} class:off={!r.enabled} data-setting={it.name}>
        <span class="words">{label(it.name)}</span>
        <span class="value">{W.level(it.name, Number(it.value))}</span>
        <input
          class="range"
          type="range"
          min={it.min}
          max={it.max}
          step={(it.max - it.min) / 24}
          value={it.value}
          style:--v="{((Number(it.value) - it.min) / (it.max - it.min || 1)) * 100}%"
          aria-label={label(it.name)}
          disabled={!r.enabled}
          onchange={(e) => void set(it.name, e.currentTarget.value)}
        />
      </div>
    {/if}
  {/if}
{/snippet}

<style>
  /* The title's words where a category's start, inside its pill. */
  :global(.page:has(> .framed) > .page-head h1) {
    padding-left: 14px;
  }
  /* The pages as chips under the title, as the desktop's; the one open raised on the paper. Without the
     room for a rail, the block is the column, in the top bar's middle. */
  :global(.page:has(> .framed)) {
    width: min(720px, var(--pill-w));
  }
  .tabs {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    margin: -4px 0 6px;
  }
  .column {
    max-width: 720px;
  }
  @media (min-width: 1280px) {
    :global(.page:has(> .framed)) {
      width: var(--pill-w);
    }
    .framed {
      display: grid;
      grid-template-columns: minmax(0, 1fr) minmax(0, 720px);
      column-gap: 40px;
      align-items: start;
    }
    /* Its first row level with the column's first header, whose words stand 22 px down (`.group-label`). */
    .tabs {
      position: sticky;
      top: var(--top-h);
      flex-direction: column;
      flex-wrap: nowrap;
      gap: 2px;
      margin: 12px 0 0;
    }
    .tab {
      height: 36px;
      border-radius: 18px;
    }
  }
  .tab {
    display: flex;
    align-items: center;
    height: 34px;
    padding: 0 14px;
    border-radius: 17px;
    border: 1px solid transparent;
    color: var(--ink2);
    font-size: 14px;
    font-weight: 500;
  }
  .tab:hover {
    color: var(--ink);
  }
  .tab.on {
    color: var(--ink);
    background: var(--surface);
    border-color: var(--hair);
  }
  .root {
    margin: 10px -4px 0;
  }
  /* As chips, the desktop's: 16 px under the title, 12 over what the page shows, its names at 13 px on one
     line wider than the column, centered over it. */
  @media (min-width: 720px) and (max-width: 1279px) {
    .tabs {
      margin: 0 -120px 12px;
    }
    .tab {
      font-size: 13px;
    }
  }
  .group {
    margin: 0 -4px;
  }
  .under {
    padding-left: 34px;
  }
  /* A profile's row: its mark, its words, the check on the one in use and its menu; the hairline from where
     its words start. A click anywhere else on it opens it. */
  .profile {
    padding-top: 10px;
    padding-bottom: 10px;
    padding-right: 4px;
  }
  .profile::after {
    left: 68px;
  }
  .profile:has(.opens:hover:enabled) {
    background: var(--hover);
  }
  .opens {
    flex: 1;
    min-width: 0;
    display: flex;
    text-align: left;
  }
  /* The one in use opens nothing, and stays in ink. */
  .opens:disabled {
    opacity: 1;
  }
  .opens::before {
    content: "";
    position: absolute;
    inset: 0;
  }
  .mark,
  .profile .glyph {
    position: relative;
    z-index: 1;
  }
  .mark {
    display: grid;
    border-radius: 50%;
  }
  .in-use {
    display: grid;
    color: var(--accent);
  }
  .in-use :global(.icon) {
    width: 20px;
    height: 20px;
  }
  .off {
    opacity: 0.38;
  }
  .note {
    margin: 0;
    padding: 0 14px 12px;
    font-size: 13px;
    color: var(--ink2);
  }
  .level {
    flex-wrap: wrap;
  }
  .stored {
    flex-wrap: wrap;
    row-gap: 8px;
  }
  .stored .words {
    flex-basis: 100%;
  }
  .bar {
    display: flex;
    gap: 2px;
    width: 100%;
    height: 10px;
    border-radius: 5px;
    overflow: hidden;
    background: var(--line);
  }
  .bar i {
    min-width: 0;
  }
  .parts {
    display: flex;
    flex-direction: column;
    gap: 2px;
    font-size: 12px;
    color: var(--ink2);
  }
  .part {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .part i {
    width: 8px;
    height: 8px;
    border-radius: 50%;
  }
  .level .range {
    width: 100%;
  }
</style>
