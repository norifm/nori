<script lang="ts">
  // Every menu on a wide window (and a phone's menus about no one thing): one surface on the paper with a
  // hairline edge and no backdrop, beside the button that opened it or at the pointer, turned or moved to
  // stay inside the window. Its head (a song's cover, title and line, its seal beside them), the bare
  // glyphs on a line under it, then its rows, each a hairline from the next and each group a full line from
  // the next; the chosen one ticked. Its corners are the tile's; what sits in them is round by the tile's
  // corner less the inset (`--inner`). A click outside or Escape closes it.
  import { ui } from "../state/ui.svelte";
  import Cover from "./Cover.svelte";
  import Icon from "./Icon.svelte";
  import type { Item } from "../state/ui.svelte";

  const m = $derived(ui.open!);
  let menu = $state<HTMLElement>();
  let place = $state("visibility:hidden");
  const EDGE = 8;
  // Placed once drawn: below a button and level with its right edge, or with its top left at the pointer;
  // turned up or left where the window ends.
  $effect(() => {
    if (!menu) return;
    const { width: w, height: h } = menu.getBoundingClientRect();
    const a = m.at;
    let x = a.point ? a.left : a.right - w;
    let y = a.point ? a.top : a.bottom + 4;
    if (a.point && x + w > innerWidth - EDGE) x = a.left - w;
    if (y + h > innerHeight - EDGE) y = a.point ? a.top - h : a.top - h - 4;
    x = Math.min(Math.max(EDGE, x), innerWidth - w - EDGE);
    y = Math.min(Math.max(EDGE, y), innerHeight - h - EDGE);
    place = `left:${x}px;top:${y}px`;
  });
  $effect(() => menu?.querySelector<HTMLElement>("button:not(:disabled)")?.focus({ preventScroll: true }));
  const close = () => (ui.open = null);
  // The seal just pressed on: it is stamped down.
  let stamped = $state<Item>();
</script>

<svelte:window onpointerdown={(e) => !menu?.contains(e.target as Node) && close()} onkeydown={(e) => e.key === "Escape" && close()} onhashchange={close} onresize={close} />

{#snippet row(it: Item)}
  <button
    class="popover-item menu-item"
    role="menuitem"
    disabled={it.disabled}
    onclick={() => ui.choose(it)}>{#if it.icon}<Icon name={it.icon} />{/if}<span class="clip">{it.label}</span>{#if it.chosen}<Icon name="check" />{/if}</button
  >
{/snippet}

{#snippet glyph(it: Item)}
  <button
    class="glyph quiet small"
    class:on={it.lit}
    class:stamp={it === stamped && it.lit}
    aria-label={it.label}
    title={it.label}
    disabled={it.disabled}
    onclick={() => (stamped = ui.choose(it))}><Icon name={it.icon ?? "more-horiz"} /></button
  >
{/snippet}

<div class="popover drop" role="menu" aria-label={m.head?.title} bind:this={menu} style={place}>
  {#if m.head}
    <div class="head">
      {#if m.head.cover !== undefined}<span class="art"><Cover src={m.head.cover || undefined} radius="var(--inner)" /></span>{/if}
      <span class="words"><span class="title clip">{m.head.title}</span>{#if m.head.line}<span class="sub clip">{m.head.line}</span>{/if}</span>
      {#each m.head.glyphs ?? [] as it, i (i)}{@render glyph(it)}{/each}
    </div>
  {/if}
  {#if m.tools.length || m.head?.quality}<div class="tools"><span class="quality clip">{m.head?.quality ?? ""}</span>{#each m.tools as it, i (i)}{@render glyph(it)}{/each}</div>{/if}
  {#each m.groups as g, n (n)}
    {#if n}<span class="split"></span>{/if}
    {#each g as it, i (i)}{@render row(it)}{/each}
  {/each}
</div>

<style>
  .drop {
    position: fixed;
    z-index: 70;
    width: 260px;
    max-height: calc(100vh - 16px);
    overflow-y: auto;
  }
  .head {
    position: relative;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 0 4px 6px 0;
    margin-bottom: 1px;
  }
  .head::after {
    content: "";
    position: absolute;
    left: -6px;
    right: -6px;
    bottom: 0;
    height: 1px;
    background: var(--line);
  }
  .art {
    width: 40px;
    flex: none;
  }
  .words {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .title {
    font-size: 15px;
    line-height: 20px;
    font-weight: 600;
  }
  .sub {
    font-size: 13px;
  }
  /* A full line between groups and under the glyphs, edge to edge as the head's. */
  .split,
  .tools {
    position: relative;
  }
  .split {
    display: block;
    height: 7px;
  }
  .split::after,
  .tools::after {
    content: "";
    position: absolute;
    left: -6px;
    right: -6px;
    top: 3px;
    height: 1px;
    background: var(--line);
  }
  .tools {
    display: flex;
    align-items: center;
    min-height: 36px;
    padding: 0 0 7px 10px;
  }
  .quality {
    flex: 1;
    font-size: 13px;
    color: var(--ink2);
  }
  .tools::after {
    top: auto;
    bottom: 3px;
  }
  .glyph.small {
    width: 36px;
    height: 36px;
  }
  .glyph.small :global(.icon) {
    width: 20px;
    height: 20px;
  }
</style>
