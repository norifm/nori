<script lang="ts">
  // A sheet of what can be done with something (a song): its head as the row it came from looked, then
  // its actions as grouped rows, each with its glyph, and those wanted less often folded under More. Opaque
  // paper on the surface with a hairline edge, no shadow: from the bottom on a phone, in the middle on a
  // wide screen. A click outside or Escape closes it.
  import { ui, type Item } from "../state/ui.svelte";
  import { W } from "../words";
  import Cover from "./Cover.svelte";
  import Icon from "./Icon.svelte";

  let dialog = $state<HTMLDialogElement>();
  const sheet = $derived(ui.sheet!);
  let more = $state(false);
  $effect(() => dialog?.showModal());
  const close = () => (ui.sheet = null);
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

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
<dialog class="menu sheet" class:wide={ui.wide} bind:this={dialog} onclose={close} onclick={backdrop} aria-label={sheet.head.title}>
  {#if !ui.wide}<span class="handle"></span>{/if}
  <header class="head">
    {#if sheet.head.cover !== undefined}<span class="art"><Cover src={sheet.head.cover || undefined} /></span>{/if}
    <span class="words">
      <span class="title clip">{sheet.head.title}</span>
      {#if sheet.head.line}<span class="sub clip">{sheet.head.line}</span>{/if}
    </span>
  </header>
  <div class="group">
    {#each sheet.items as it, i (i)}{@render row(it)}{/each}
    {#if sheet.more.length}
      <button class="group-row" aria-expanded={more} onclick={() => (more = !more)}><Icon name="more-horiz" /><span class="words">{W.more}</span><span class="turn" class:open={more}><Icon name="nav-arrow-down" /></span></button>
      {#if more}{#each sheet.more as it, i (i)}{@render row(it)}{/each}{/if}
    {/if}
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
  .sheet.wide {
    margin: auto;
    width: min(420px, calc(100vw - 32px));
    padding: 16px;
    border: var(--hairline) solid var(--hair);
    border-radius: var(--r-sheet);
    animation: none;
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
  .turn {
    display: grid;
    color: var(--ink2);
    transition: transform 220ms;
  }
  .turn.open {
    transform: rotate(180deg);
  }
  .turn :global(.icon) {
    width: 22px;
    height: 22px;
  }
</style>
