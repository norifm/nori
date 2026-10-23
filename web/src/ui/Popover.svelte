<script lang="ts">
  // A menu dropped from the button that opened it, every menu the same: raised on the surface with a
  // hairline edge and the tile's corners, no shadow; the chosen row marked with a quiet check. A click
  // outside or Escape closes it.
  import { ui } from "../state/ui.svelte";
  import Icon from "./Icon.svelte";

  const p = $derived(ui.popover!);
  let menu = $state<HTMLElement>();
  /** Under the button, its right edge on the button's, kept inside the window. */
  const place = $derived.by(() => {
    const below = p.at.bottom + 4 + 44 * p.items.length + 8 < innerHeight;
    const right = Math.max(8, innerWidth - p.at.right);
    return below ? `top:${p.at.bottom + 4}px;right:${right}px` : `bottom:${innerHeight - p.at.top + 4}px;right:${right}px`;
  });
  $effect(() => menu?.querySelector<HTMLElement>("button")?.focus());
  const close = () => (ui.popover = null);
</script>

<svelte:window onpointerdown={(e) => !menu?.contains(e.target as Node) && close()} onkeydown={(e) => e.key === "Escape" && close()} onhashchange={close} />

<div class="popover drop" role="menu" bind:this={menu} style={place}>
  {#each p.items as it, i (i)}
    <button
      class="popover-item"
      role="menuitem"
      disabled={it.disabled}
      onclick={() => {
        close();
        void it.run();
      }}><span class="clip">{it.label}</span>{#if it.chosen}<Icon name="check" />{/if}</button
    >
  {/each}
</div>

<style>
  .drop {
    position: fixed;
    z-index: 70;
  }
</style>
