<script lang="ts">
  // A row of the devices panel, as the desktop's: its glyph in a disc (raised while `lit`), its name and a
  // line under it, and what follows at its end.
  import type { Snippet } from "svelte";
  import Icon from "./Icon.svelte";

  let { icon, title, sub = "", lit = false, onclick, end, ...rest }: { icon: string; title: string; sub?: string; lit?: boolean; onclick?: () => void; end?: Snippet; [k: string]: unknown } = $props();
</script>

<button class="panel-row" class:lit aria-pressed={lit} {onclick} {...rest}>
  <span class="disc"><Icon name={icon} /></span>
  <span class="words"><span class="title clip">{title}</span>{#if sub}<span class="sub clip">{sub}</span>{/if}</span>
  {@render end?.()}
</button>

<style>
  .panel-row {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    min-height: 52px;
    padding: 0 8px;
    border-radius: 10px;
    text-align: left;
  }
  .panel-row:hover {
    background: var(--hover);
  }
  .disc {
    display: grid;
    place-items: center;
    width: 34px;
    height: 34px;
    flex: none;
    border-radius: 50%;
    background: color-mix(in srgb, var(--ink) 7%, transparent);
  }
  .lit .disc {
    background: color-mix(in srgb, var(--ink) 14%, transparent);
  }
  .disc :global(.icon) {
    width: 18px;
    height: 18px;
  }
  .words {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .title {
    font-size: 13px;
    font-weight: 600;
  }
  .lit .title {
    color: var(--accent);
  }
  .sub {
    font-size: 11px;
    color: var(--ink2);
  }
  .panel-row > :global(.icon) {
    width: 18px;
    height: 18px;
    color: var(--accent);
  }
</style>
