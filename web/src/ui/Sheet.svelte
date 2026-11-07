<script lang="ts">
  // A phone's menu about one thing (a song), as Android's sheet: its head as the row it came from looked with
  // its seal beside the title, the bare glyphs on a line under it, then its actions as grouped rows, each
  // with its glyph, every group on one plate.
  // Opaque paper on the surface with a hairline edge, no shadow, from the bottom. A click outside or Escape
  // closes it.
  import { ui, type Item } from "../state/ui.svelte";
  import Cover from "./Cover.svelte";
  import Icon from "./Icon.svelte";

  let dialog = $state<HTMLDialogElement>();
  const sheet = $derived(ui.open!);
  $effect(() => dialog?.showModal());
  const close = () => (ui.open = null);
  /** A click on the backdrop, not on the sheet's own padding (both target the dialog). */
  function backdrop(e: MouseEvent) {
    const r = dialog!.getBoundingClientRect();
    if (e.target === dialog && (e.clientX < r.left || e.clientX > r.right || e.clientY < r.top || e.clientY > r.bottom)) close();
  }
</script>

{#snippet row(it: Item)}
  <button
    class="group-row menu-item"
    disabled={it.disabled}
    onclick={() => {
      close();
      void it.run();
    }}>{#if it.icon}<Icon name={it.icon} />{/if}<span class="words">{it.label}</span></button
  >
{/snippet}

{#snippet glyph(it: Item)}
  <button
    class="glyph quiet"
    class:on={it.lit}
    aria-label={it.label}
    title={it.label}
    disabled={it.disabled}
    onclick={() => {
      close();
      void it.run();
    }}><Icon name={it.icon ?? "more-horiz"} /></button
  >
{/snippet}

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
<dialog class="menu sheet" bind:this={dialog} onclose={close} onclick={backdrop} aria-label={sheet.head?.title}>
  <span class="handle"></span>
  <header class="head">
    {#if sheet.head?.cover !== undefined}<span class="art"><Cover src={sheet.head?.cover || undefined} /></span>{/if}
    <span class="words">
      <span class="title clip">{sheet.head?.title}</span>
      {#if sheet.head?.line}<span class="sub clip">{sheet.head?.line}</span>{/if}
    </span>
    {#each sheet.head?.glyphs ?? [] as it, i (i)}{@render glyph(it)}{/each}
  </header>
  {#if sheet.tools.length || sheet.head?.quality}<div class="tools"><span class="quality clip">{sheet.head?.quality ?? ""}</span>{#each sheet.tools as it, i (i)}{@render glyph(it)}{/each}</div>{/if}
  <div class="group">
    {#each sheet.groups.flat() as it, i (i)}{@render row(it)}{/each}
  </div>
</dialog>

<style>
  .sheet {
    margin: auto auto 0;
    width: 100%;
    max-width: 560px;
    max-height: 86vh;
    padding: 8px 16px calc(16px + env(safe-area-inset-bottom));
    border: 0;
    border-top: var(--hairline) solid var(--hair);
    border-radius: var(--r-sheet) var(--r-sheet) 0 0;
    background: var(--surface);
    color: var(--ink);
    animation: rise 240ms cubic-bezier(0.05, 0.7, 0.1, 1);
  }
  .sheet::backdrop {
    background: var(--scrim);
  }
  @keyframes rise {
    from {
      transform: translateY(40px);
      opacity: 0;
    }
  }
  .handle {
    display: block;
    width: 36px;
    height: 4px;
    border-radius: 2px;
    margin: 0 auto 12px;
    background: color-mix(in srgb, var(--ink) 20%, transparent);
  }
  .head {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 4px 4px 12px;
  }
  .art {
    width: 52px;
    flex: none;
  }
  .words {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .title {
    font-size: 16px;
    line-height: 22px;
    font-weight: 600;
  }
  .group-row:disabled {
    opacity: 0.38;
  }
  .tools {
    display: flex;
    align-items: center;
    min-height: 48px;
    margin: -4px 0 8px;
    padding-left: 4px;
  }
  .quality {
    flex: 1;
    font-size: 14px;
    color: var(--ink2);
  }
</style>
