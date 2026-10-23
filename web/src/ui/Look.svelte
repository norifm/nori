<script lang="ts">
  // The look, as Android's Appearance has it: the four season tiles (each its kanji over its paper, accent
  // and "on" colors, the one worn raised; picking one stops following the calendar), Follow the calendar,
  // then System, Day, Night or Midnight. The core keeps the settings and hands out the colors.
  import { ask } from "../rpc";
  import { refreshTheme, worn } from "../state/theme.svelte";
  import { W } from "../words";
  import Icon from "./Icon.svelte";

  type Look = { season: string; theme: string };
  let look = $state<Look>({ season: "FOLLOW", theme: "SYSTEM" });
  const read = async () => {
    const v = JSON.parse(await ask<string>("settings", "season,theme")) as Look;
    look = { season: v.season.toUpperCase(), theme: v.theme.toUpperCase() };
  };
  void read();

  const kanji: Record<string, string> = { SPRING: "kanji-spring", SUMMER: "kanji-summer", AUTUMN: "kanji-autumn", WINTER: "kanji-winter" };
  const themes = ["SYSTEM", "LIGHT", "DARK", "MIDNIGHT"];
  const following = $derived(look.season === "FOLLOW");
  /** The tile worn: the calendar's while following. */
  const shown = $derived(following ? worn.season : look.season);
  const named = (season: string) => W.seasons[season] ?? season;

  async function set(name: keyof Look, value: string) {
    look[name] = value;
    await ask("setting", name, value);
    await refreshTheme();
  }
</script>

<div class="look">
  <div class="group-label">{W.season}</div>
  <div class="tiles" role="radiogroup" aria-label={W.season}>
    {#each worn.tiles as t (t.season)}
      {@const on = shown === t.season}
      <button class="tile" class:on role="radio" aria-checked={on} aria-label={named(t.season)} data-season={t.season} onclick={() => (!on || following) && void set("season", t.season)}>
        <Icon name={kanji[t.season]} />
        <span class="dots"><i class="paper" style:background={t.page}></i><i style:background={t.accent}></i><i style:background={t.on}></i></span>
      </button>
    {/each}
  </div>
  <div class="group">
    <label class="group-row" data-setting="season">
      <span class="words"><span>{W.seasonFollow}</span>{#if following}<span class="sub">{W.seasonNext(named(worn.next), worn.nextMonth, worn.nextDay)}</span>{/if}</span>
      <input type="checkbox" class="switch" checked={following} onchange={(e) => void set("season", e.currentTarget.checked ? "FOLLOW" : worn.season)} />
    </label>
  </div>
  <div class="group-label">{W.sections.Theme}</div>
  <div class="group">
    <div class="group-row chips-row" data-setting="theme">
      <span class="words">{W.settingNames.theme}</span>
      <div class="choices" role="radiogroup" aria-label={W.settingNames.theme}>
        {#each themes as id (id)}
          <button class="chip" class:on={look.theme === id} role="radio" aria-checked={look.theme === id} data-theme={id} onclick={() => look.theme !== id && void set("theme", id)}>{W.choice("theme", id)}</button>
        {/each}
      </div>
    </div>
  </div>
</div>

<style>
  .tiles {
    display: flex;
    gap: 10px;
    margin-bottom: 10px;
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
  /* In the top bar's menu the tiles stand on the menu's own paper: the worn one is raised on the page. */
  :global(.popover) .tile.on {
    background: var(--page);
  }
  :global(.popover) .group-label:first-child {
    margin-top: 6px;
  }
</style>
