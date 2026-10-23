<script lang="ts" module>
  import { W } from "../words";

  /** The tabs, in Android's order: their page, glyph and name. */
  export const tabs: [string, string, string][] = [
    ["home", "home-simple", W.home],
    ["library", "album-list", W.library],
    ["settings", "settings", W.settings],
    ["search", "search", W.search],
  ];
</script>

<script lang="ts">
  // A phone's tabs: the bottom row of the chrome, each an equal share of it, its glyph quiet until it is
  // the current one, which takes the accent. Their names are for screen readers.
  import { href, route } from "../state/route.svelte";
  import Icon from "./Icon.svelte";
</script>

<nav class="tabs" id="nav">
  {#each tabs as [top, icon, label] (top)}
    <a href={href(top)} class:on={route.tab === top} data-top={top} aria-label={label} aria-current={route.tab === top ? "page" : undefined}><Icon name={icon} /></a>
  {/each}
</nav>

<style>
  .tabs {
    height: 50px;
    display: flex;
    padding: 0 6px;
  }
  a {
    flex: 1;
    display: grid;
    place-items: center;
    color: color-mix(in srgb, var(--ink) 65%, transparent);
    transition: color 220ms;
  }
  a :global(.icon) {
    width: 21px;
    height: 21px;
  }
  a.on {
    color: var(--accent);
  }
</style>
