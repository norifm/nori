<script lang="ts">
  // A phone's tabs: the bottom row of the bottom bar, as the Android app has them.
  import { href, route } from "../state/route.svelte";
  import { W } from "../words";
  import Icon from "./Icon.svelte";

  const tabs: [string, string, string][] = [
    ["home", "home-simple", W.home],
    ["library", "album-list", W.library],
    ["search", "search", W.search],
    ["settings", "settings", W.settings],
  ];
</script>

<nav class="tabs" id="nav">
  {#each tabs as [top, icon, label] (top)}
    <a href={href(top)} class:on={route.tab === top} data-top={top}><Icon name={icon} /><span>{label}</span></a>
  {/each}
</nav>

<style>
  .tabs {
    height: 56px;
    display: flex;
  }
  a {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 2px;
    font-size: 10.5px;
    font-weight: 500;
    color: var(--ink2);
  }
  a :global(.icon) {
    width: 24px;
    height: 24px;
  }
  a.on {
    color: var(--accent);
    font-weight: 700;
  }
</style>
