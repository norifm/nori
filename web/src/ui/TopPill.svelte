<script lang="ts">
  // The top bar on a wide screen: the tabs, in their order, as one outlined pill with their names (the
  // current one in the accent), and in the Search tab's place the search field, which lists the best
  // matches under it as you type.
  import { href, route } from "../state/route.svelte";
  import Icon from "./Icon.svelte";
  import SearchBox from "./SearchBox.svelte";
  import { tabs } from "./TabBar.svelte";
</script>

<header class="top">
  <span></span>
  <nav class="chrome tabs" id="nav">
    {#each tabs.filter(([top]) => top !== "search") as [top, icon, label] (top)}
      <a class="tab" class:on={route.tab === top} href={href(top)} data-top={top} aria-current={route.tab === top ? "page" : undefined}><Icon name={icon} /><span>{label}</span></a>
    {/each}
  </nav>
  <div class="right"><SearchBox /></div>
</header>

<style>
  .top {
    position: fixed;
    top: 14px;
    left: 16px;
    right: 16px;
    z-index: 30;
    display: grid;
    grid-template-columns: 1fr auto 1fr;
    align-items: center;
    gap: 12px;
    height: 50px;
  }
  .tabs {
    display: flex;
    height: 50px;
    padding: 0 6px;
    border-radius: 25px;
  }
  .tab {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 16px;
    color: color-mix(in srgb, var(--ink) 65%, transparent);
    font-size: 14px;
    font-weight: 500;
    white-space: nowrap;
    transition: color 220ms;
  }
  .tab :global(.icon) {
    width: 21px;
    height: 21px;
  }
  .tab:hover {
    color: var(--ink);
  }
  .tab.on {
    color: var(--accent);
  }
  .right {
    display: flex;
    justify-content: flex-end;
  }
</style>
