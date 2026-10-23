<script lang="ts">
  // The look: the season (the four tiles marked 春 夏 秋 冬; following the calendar, its season's is
  // marked), System, Day, Night or Midnight. The core keeps the settings and hands out the colors (state/theme.ts).
  import { ask } from "../rpc";
  import { refreshTheme, worn } from "../state/theme.svelte";
  import { W } from "../words";

  type Look = { season: string; theme: string };
  let look = $state<Look>({ season: "FOLLOW", theme: "SYSTEM" });
  const read = async () => {
    const v = JSON.parse(await ask<string>("settings", "season,theme")) as Look;
    look = { season: v.season.toUpperCase(), theme: v.theme.toUpperCase() };
  };
  void read();

  const following = $derived(look.season === "FOLLOW");
  /** The tile worn: the calendar's while following. */
  const shown = $derived(following ? worn.season : look.season);
  const named = (id: string) => W.seasons.find((s) => s[0] === id)?.[2] ?? id;

  async function set(name: keyof Look, value: string) {
    look[name] = value;
    await ask("setting", name, value);
    await refreshTheme();
  }
</script>

<div class="look">
  <div class="tiles" role="radiogroup" aria-label={W.season}>
    {#each W.seasons as [id, kanji, name] (id)}
      <button class="tile" role="radio" aria-checked={shown === id} class:on={shown === id} data-season={id} onclick={() => void set("season", id)}>
        <i>{kanji}</i><small>{name}</small>
      </button>
    {/each}
  </div>
  <label class="row">
    <span class="words"
      ><span>{W.seasonFollow}</span><span class="sub">{following ? W.seasonNow(named(worn.season), named(worn.next), worn.nextMonth, worn.nextDay) : W.seasonFollowLead}</span></span
    >
    <input type="checkbox" class="switch" checked={following} onchange={(e) => void set("season", e.currentTarget.checked ? "FOLLOW" : worn.season)} />
  </label>
  <div class="row stack">
    <span>{W.appearance}</span>
    <div class="chips" role="radiogroup" aria-label={W.appearance}>
      {#each W.appearances as [id, name] (id)}
        <button role="radio" aria-checked={look.theme === id} class:on={look.theme === id} data-theme={id} onclick={() => void set("theme", id)}>{name}</button>
      {/each}
    </div>
  </div>
</div>

<style>
  .tiles {
    display: flex;
    gap: 8px;
    margin: 4px 0 10px;
  }
  .tile {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    border: 1px solid var(--outline);
    border-radius: 14px;
    padding: 10px 4px;
    color: var(--ink2);
  }
  .tile i {
    font: 600 28px/1.2 "Shippori Mincho", "Hiragino Mincho ProN", "Yu Mincho", "Noto Serif CJK JP", serif;
    font-style: normal;
  }
  .tile small {
    font-size: 11px;
  }
  .tile.on {
    border-color: var(--accent);
    color: var(--ink);
  }
  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 9px 2px;
    border-top: 1px solid var(--hair);
    cursor: default;
  }
  .stack {
    flex-direction: column;
    align-items: flex-start;
  }
  .words {
    display: flex;
    flex-direction: column;
  }
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .chips button {
    border: 1px solid var(--outline);
    border-radius: 999px;
    padding: 4px 11px;
    font-size: 13px;
    color: var(--ink2);
  }
  .chips button.on {
    background: var(--ink);
    border-color: var(--ink);
    color: var(--page);
  }
</style>
