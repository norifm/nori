<script lang="ts">
  // A phone's tab bar, as the Android app and Apple Music have it.
  import { href, route } from "../state/route.svelte";
  import { W } from "../words";
  import Icon from "./Icon.svelte";

  const tabs: [string, string, string][] = [
    ["home", "home", W.home],
    ["library", "library", W.library],
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
    position: fixed;
    left: 0;
    right: 0;
    bottom: 0;
    z-index: 30;
    height: calc(var(--tabs-h) + env(safe-area-inset-bottom));
    padding-bottom: env(safe-area-inset-bottom);
    display: flex;
    background: var(--bar);
    border-top: 1px solid var(--hair);
  }
  a {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 3px;
    font-size: 10px;
    font-weight: 500;
    color: var(--faint);
  }
  a :global(.icon) {
    width: 24px;
    height: 24px;
  }
  a.on {
    color: var(--accent);
  }
</style>
