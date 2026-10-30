<script lang="ts">
  // Settings. On a wide screen as the desktop's: the servers first, the pages the core lays out for a
  // browser (`settings_pages`), then the plugins, as tabs under the title, the one open below them. On a phone as
  // Android's: the root lists them, each with its glyph and what it holds, and a page (`#/settings/<page>`)
  // opens on its own. A page has its sections as small-caps labels over grouped cards, each row drawn by
  // its kind and worded here. A row says only what is not obvious from its name. Appearance is the look
  // the top bar's menu has too.
  import { ask } from "../rpc";
  import { reading } from "../state/library.svelte";
  import { route, href } from "../state/route.svelte";
  import { refreshTheme } from "../state/theme.svelte";
  import { ui } from "../state/ui.svelte";
  import type { SettingRow, SettingsSection } from "../state/types";
  import { W } from "../words";
  import Icon from "../ui/Icon.svelte";
  import Look from "../ui/Look.svelte";
  import Page from "../ui/Page.svelte";
  import Plugins from "../ui/Plugins.svelte";

  const glyphs: Record<string, string> = { Playing: "play", Sound: "control-slider", Look: "palette", Lyrics: "quote-message", Library: "album-list", Servers: "server", Plugins: "view-grid", About: "info-circle" };
  let rev = $state(0);
  const page = reading("settings", async () => {
    void rev;
    const names = JSON.parse(await ask<string>("settings_pages")) as string[];
    const name = route.arg;
    const sections = names.includes(name) ? (JSON.parse(await ask<string>("settings_page", name)) as SettingsSection[]) : [];
    const profile = JSON.parse(await ask<string>("profile")) as { url: string; user: string } | null;
    return { names, sections, profile };
  });
  /** The page open: on a wide screen the first while none is asked for. */
  const name = $derived(route.arg || (ui.wide ? "Servers" : ""));
  const tabs = $derived([["Servers", W.servers], ...(page.data?.names ?? []).map((n) => [n, W.settingsPages[n]?.[0] ?? n]), ["Plugins", W.plugins]]);
  /** The page's sections the rows draw: Appearance is the look, drawn whole. */
  const drawn = $derived(name === "Look" ? [] : (page.data?.sections ?? []));

  async function set(setting: string, value: string) {
    await ask("setting", setting, value);
    if (setting === "season" || setting === "theme" || setting === "motion") await refreshTheme();
    rev++;
  }

  async function signOut() {
    await ask("logout");
    location.hash = "";
    location.reload();
  }

  const shownRows = (s: SettingsSection) => s.rows.filter((r) => r.shown);
  const label = (setting: string) => W.settingNames[setting] ?? setting;
</script>

{#if ui.wide}
  <Page title={W.settings} narrow ready={!!page.data} error={page.error}>
    <nav class="tabs" aria-label={W.settings}>
      {#each tabs as [n, label] (n)}
        <a class="tab" class:on={n === name} aria-current={n === name ? "page" : undefined} href={href("settings", n)} data-page={n}>{label}</a>
      {/each}
    </nav>
    {#if name === "Servers"}{@render serversPage()}{:else if name === "Plugins"}<Plugins />{:else}{@render settingsPage()}{/if}
  </Page>
{:else if !name}
  <Page title={W.settings} narrow ready={!!page.data} error={page.error}>
    <div class="group root">
      <a class="group-row" href={href("settings", "Servers")}><Icon name="server" /><span class="words"><span>{W.servers}</span><span class="sub">{page.data?.profile?.user ?? ""}</span></span><Icon name="nav-arrow-right" /></a>
      {#each page.data?.names ?? [] as n (n)}
        <a class="group-row" href={href("settings", n)} data-page={n}><Icon name={glyphs[n] ?? "settings"} /><span class="words"><span>{W.settingsPages[n]?.[0] ?? n}</span><span class="sub">{W.settingsPages[n]?.[1] ?? ""}</span></span><Icon name="nav-arrow-right" /></a>
      {/each}
      <a class="group-row" href={href("settings", "Plugins")}><Icon name="view-grid" /><span class="words">{W.plugins}</span><Icon name="nav-arrow-right" /></a>
    </div>
  </Page>
{:else if name === "Servers"}
  <Page title={W.servers} back narrow ready={!!page.data} error={page.error}>{@render serversPage()}</Page>
{:else if name === "Plugins"}
  <Page title={W.plugins} back narrow ready={!!page.data} error={page.error}><Plugins /></Page>
{:else}
  <Page title={W.settingsPages[name]?.[0] ?? name} back narrow ready={!!page.data} error={page.error}>{@render settingsPage()}</Page>
{/if}

{#snippet serversPage()}
  <div class="group-label">{W.accounts}</div>
  <div class="group">
    <div class="group-row"><Icon name="server" /><span class="words"><span>{page.data?.profile?.user ?? ""}</span><span class="sub">{page.data?.profile?.url ?? ""}</span></span></div>
    <button class="group-row" onclick={() => void signOut()}><Icon name="log-out" /><span class="words">{W.signOut}</span></button>
  </div>
{/snippet}

{#snippet settingsPage()}
  {#if name === "Look"}<Look />{/if}
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
  /* The pages as chips under the title, as the desktop's; the one open raised on the paper. */
  .tabs {
    display: flex;
    flex-wrap: wrap;
    justify-content: center;
    gap: 4px;
    margin: -4px 0 6px;
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
  .group {
    margin: 0 -4px;
  }
  .under {
    padding-left: 34px;
  }
  .off {
    opacity: 0.5;
  }
  .note {
    margin: 0;
    padding: 0 14px 12px;
    font-size: 13px;
    color: var(--ink2);
  }
  /* Chips under their name, the same room above and below them. */
  .chips-row {
    flex-direction: column;
    align-items: stretch;
    gap: 10px;
    padding-top: 12px;
    padding-bottom: 12px;
  }
  .choices {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
  }
  .level {
    flex-wrap: wrap;
  }
  .level .range {
    width: 100%;
  }
</style>
