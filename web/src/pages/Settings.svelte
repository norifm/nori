<script lang="ts">
  // Settings, as Android's: the root lists the pages the core lays out for a browser (`settings_pages`),
  // each with its glyph and what it holds, then the account and About; a page (`#/settings/<page>`) has its
  // sections as small-caps labels over grouped cards, each row drawn by its kind and worded here. A row says
  // only what is not obvious from its name. Appearance draws the season as its four tiles.
  import { ask } from "../rpc";
  import { reading } from "../state/library.svelte";
  import { route, href } from "../state/route.svelte";
  import { refreshTheme, worn } from "../state/theme.svelte";
  import { ui } from "../state/ui.svelte";
  import type { SettingRow, SettingsSection } from "../state/types";
  import { W } from "../words";
  import Icon from "../ui/Icon.svelte";
  import Page from "../ui/Page.svelte";

  const glyphs: Record<string, string> = { Playing: "play", Sound: "control-slider", Look: "palette", Lyrics: "emoji-sing-right-note", Library: "album-list", Account: "server", About: "info-circle" };
  let rev = $state(0);
  const page = reading("settings", async () => {
    void rev;
    const names = JSON.parse(await ask<string>("settings_pages")) as string[];
    const name = route.arg;
    const sections = names.includes(name) ? (JSON.parse(await ask<string>("settings_page", name)) as SettingsSection[]) : [];
    const profile = JSON.parse(await ask<string>("profile")) as { url: string; user: string } | null;
    return { names, sections, profile };
  });
  const name = $derived(route.arg);

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
  const isSeason = (r: SettingRow) => r.item.kind === "setting" && r.item.name === "season";
  const label = (setting: string) => W.settingNames[setting] ?? setting;
  /** The season worn: the calendar's while following. */
  const wornSeason = (value: string) => (value === "FOLLOW" ? worn.season : value);
  const named = (season: string) => W.seasons[season] ?? season;
</script>

{#if !name}
  <Page title={W.settings} ready={!!page.data} error={page.error}>
    <div class="group root">
      {#each page.data?.names ?? [] as n (n)}
        <a class="group-row" href={href("settings", n)} data-page={n}><Icon name={glyphs[n] ?? "settings"} /><span class="words"><span>{W.settingsPages[n]?.[0] ?? n}</span><span class="sub">{W.settingsPages[n]?.[1] ?? ""}</span></span><Icon name="nav-arrow-right" /></a>
      {/each}
      <a class="group-row" href={href("settings", "Account")}><Icon name="server" /><span class="words"><span>{W.account}</span><span class="sub">{page.data?.profile?.user ?? ""}</span></span><Icon name="nav-arrow-right" /></a>
    </div>
    <p class="about">{W.aboutLead}</p>
  </Page>
{:else if name === "Account"}
  <Page title={W.account} back ready={!!page.data} error={page.error}>
    <div class="group-label">{W.thisServer}</div>
    <div class="group">
      <div class="group-row"><Icon name="server" /><span class="words"><span>{page.data?.profile?.user ?? ""}</span><span class="sub">{page.data?.profile?.url ?? ""}</span></span></div>
      <button class="group-row" onclick={() => void signOut()}><Icon name="log-out" /><span class="words">{W.signOut}</span></button>
    </div>
  </Page>
{:else}
  <Page title={W.settingsPages[name]?.[0] ?? name} back ready={!!page.data} error={page.error}>
    {#each page.data?.sections ?? [] as sec (sec.id)}
      {@const seasons = shownRows(sec).filter(isSeason)}
      {@const rest = shownRows(sec).filter((r) => !isSeason(r))}
      {#each seasons as r (r)}{@render season(r)}{/each}
      {#if rest.length}
        <div class="group-label">{W.sections[sec.id] ?? sec.id}</div>
        <div class="group">
          {#each rest as r, i (i)}{@render row(r)}{/each}
        </div>
      {/if}
    {/each}
  </Page>
{/if}

{#snippet season(r: SettingRow)}
  {@const it = r.item as Extract<SettingRow["item"], { kind: "setting" }>}
  {@const following = it.value === "FOLLOW"}
  <div class="group-label">{W.season}</div>
  <div class="tiles" role="radiogroup" aria-label={W.season}>
    {#each worn.tiles as t (t.season)}
      {@const on = wornSeason(it.value) === t.season}
      <button class="tile" class:on role="radio" aria-checked={on} aria-label={named(t.season)} data-season={t.season} onclick={() => !on || following ? void set("season", t.season) : undefined}>
        <Icon name="kanji-{t.season.toLowerCase()}" />
        <span class="dots"><i class="paper" style:background={t.page}></i><i style:background={t.accent}></i><i style:background={t.on}></i></span>
      </button>
    {/each}
  </div>
  <div class="group follow">
    <label class="group-row" data-setting="season">
      <span class="words"><span>{W.seasonFollow}</span>{#if following}<span class="sub">{W.seasonNext(named(worn.next), worn.nextMonth, worn.nextDay)}</span>{/if}</span>
      <input type="checkbox" class="switch" checked={following} onchange={(e) => void set("season", e.currentTarget.checked ? "FOLLOW" : worn.season)} />
    </label>
  </div>
{/snippet}

{#snippet row(r: SettingRow)}
  {#if r.item.kind === "note"}
    <p class="note">{W.notes[r.item.note] ?? ""}</p>
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
  .root {
    margin: 10px -4px 0;
  }
  .group {
    margin: 0 -4px;
  }
  .about {
    color: var(--ink2);
    font-size: 13px;
    margin: 32px 12px 0;
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
  /* The seasons: each its kanji over its paper, accent and "on" colors; the one worn raised on the surface. */
  .tiles {
    display: flex;
    gap: 10px;
    margin: 0 -4px;
  }
  .tile {
    flex: 1;
    height: 88px;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 8px;
    border-radius: var(--r-tile);
    border: var(--hairline) solid transparent;
    color: var(--ink2);
  }
  .tile.on {
    background: var(--surface);
    border-color: var(--hair);
    color: var(--ink);
  }
  .tile :global(.icon) {
    width: 30px;
    height: 30px;
  }
  .dots {
    display: flex;
    gap: 5px;
  }
  .dots i {
    width: 8px;
    height: 8px;
    border-radius: 50%;
  }
  .dots .paper {
    box-shadow: inset 0 0 0 var(--hairline) var(--hair);
  }
  .follow {
    margin-top: 10px;
  }
</style>
