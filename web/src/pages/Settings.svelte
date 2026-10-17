<script lang="ts">
  // Settings: the pages the core lays out for a browser (`settings_page`), each row drawn by its kind and
  // worded here; then the jam, the account and what this is.
  import { ask } from "../rpc";
  import { reading } from "../state/library.svelte";
  import { player } from "../state/player.svelte";
  import type { SettingRow, SettingsSection } from "../state/types";
  import { W } from "../words";
  import JamPanel from "../ui/JamPanel.svelte";
  import Page from "../ui/Page.svelte";

  let rev = $state(0);
  const page = reading("settings", async () => {
    void rev;
    const names = JSON.parse(await ask<string>("settings_pages")) as string[];
    const pages = await Promise.all(names.map(async (n) => [n, JSON.parse(await ask<string>("settings_page", n))] as [string, SettingsSection[]]));
    const profile = JSON.parse(await ask<string>("profile")) as { url: string; user: string } | null;
    return { pages, profile };
  });

  async function set(name: string, value: string) {
    await ask("setting", name, value);
    rev++;
    if (player.up) void player.readSound();
  }

  async function signOut() {
    await ask("logout");
    location.hash = "";
    location.reload();
  }

  const label = (name: string) => W.settingNames[name]?.[0] ?? name;
  const lead = (name: string) => W.settingNames[name]?.[1] ?? "";
  const shownRows = (s: SettingsSection) => s.rows.filter((r) => r.shown);
</script>

<Page title={W.settings} ready={!!page.data} error={page.error}>
  {#each page.data?.pages ?? [] as [name, sections] (name)}
    <h2 class="section">{W.settingsPages[name] ?? name}</h2>
    {#each sections as sec (sec.id)}
      {#if shownRows(sec).length}
        <h3>{W.sections[sec.id] ?? sec.id}</h3>
        <div class="group">
          {#each shownRows(sec) as r, i (i)}
            {@render row(r)}
          {/each}
        </div>
      {/if}
    {/each}
  {/each}
  <h2 class="section">{W.jam}</h2>
  <div class="group pad"><JamPanel /></div>
  <h2 class="section">{W.account}</h2>
  <div class="group">
    <div class="setting">
      <span class="sub">{page.data?.profile ? W.signedInAs(page.data.profile.user, page.data.profile.url) : ""}</span>
      <button class="pill" onclick={() => void signOut()}>{W.signOut}</button>
    </div>
  </div>
  <p class="about">{W.aboutLead}</p>
</Page>

{#snippet row(r: SettingRow)}
  {#if r.item.kind === "note"}
    <p class="note" class:under={r.depth > 0}>{W.notes[r.item.note] ?? ""}</p>
  {:else}
    {@const it = r.item}
    <div class="setting" class:under={r.depth > 0} class:off={!r.enabled} data-setting={it.name}>
      <div class="words">
        <span class="title">{label(it.name)}</span>
        {#if lead(it.name)}<span class="sub">{lead(it.name)}</span>{/if}
      </div>
      {#if it.type === "Switch"}
        <input type="checkbox" class="switch" aria-label={label(it.name)} checked={it.value === "true"} disabled={!r.enabled} onchange={(e) => void set(it.name, e.currentTarget.checked ? "true" : "false")} />
      {:else if it.type === "Choice" && it.chips}
        <div class="chips" role="radiogroup" aria-label={label(it.name)}>
          {#each it.options as o (o)}
            <button role="radio" class:on={o === it.value} aria-checked={o === it.value} disabled={!r.enabled} onclick={() => void set(it.name, o)}>{W.choice(it.name, o)}</button>
          {/each}
        </div>
      {:else if it.type === "Choice"}
        <select class="field pick" aria-label={label(it.name)} disabled={!r.enabled} onchange={(e) => void set(it.name, e.currentTarget.value)}>
          {#each it.options as o (o)}<option value={o} selected={Number(o) === Number(it.value) || o === it.value}>{W.choice(it.name, o)}</option>{/each}
        </select>
      {:else if it.type === "Level"}
        <div class="level">
          <input type="range" min={it.min} max={it.max} step={(it.max - it.min) / 24} value={it.value} aria-label={label(it.name)} disabled={!r.enabled} onchange={(e) => void set(it.name, e.currentTarget.value)} />
          <span class="sub">{W.level(it.name, Number(it.value))}</span>
        </div>
      {/if}
    </div>
  {/if}
{/snippet}

<style>
  h3 {
    font-size: 13px;
    font-weight: 600;
    color: var(--dim);
    margin: 18px 4px 8px;
  }
  .group {
    background: var(--fill);
    border-radius: 12px;
    max-width: 760px;
  }
  .pad {
    padding: 16px;
  }
  .setting {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    padding: 12px 16px;
    min-height: 52px;
    border-top: 1px solid var(--hair);
  }
  .setting:first-child {
    border-top: 0;
  }
  .under {
    padding-left: 34px;
  }
  .off {
    opacity: 0.5;
  }
  .words {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .title {
    font-size: 15px;
  }
  .note {
    margin: 0;
    padding: 0 16px 12px;
    font-size: 12px;
    color: var(--dim);
  }
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    background: var(--fill);
    padding: 2px;
    border-radius: 9px;
    flex: none;
  }
  .chips button {
    padding: 4px 10px;
    border-radius: 7px;
    font-size: 13px;
    font-weight: 500;
  }
  .chips button.on {
    background: var(--raised);
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.15);
  }
  .pick {
    width: auto;
    min-width: 110px;
    height: 32px;
    font-size: 14px;
  }
  .level {
    display: flex;
    align-items: center;
    gap: 10px;
    flex: none;
  }
  .level input {
    accent-color: var(--accent);
    width: 160px;
  }
  .about {
    color: var(--faint);
    font-size: 13px;
    margin-top: 32px;
  }
  @media (max-width: 899px) {
    .setting {
      flex-wrap: wrap;
    }
  }
</style>
