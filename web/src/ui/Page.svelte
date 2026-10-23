<script lang="ts">
  // A page: its name (large on a tab's own page; beside a way back on a page opened from another), its
  // controls beside the name, then what it shows; while its read is out, a quiet word; if it failed, why.
  import type { Snippet } from "svelte";
  import { W } from "../words";
  import Icon from "./Icon.svelte";

  let { title = "", back = false, error, ready = true, head, children }: { title?: string; back?: boolean; error?: string; ready?: boolean; head?: Snippet; children: Snippet } = $props();
</script>

<div class="page">
  {#if back}
    <header class="back-title">
      <button class="glyph" aria-label={W.back} title={W.back} onclick={() => history.back()}><Icon name="arrow-left" /></button>
      <h1 class="clip">{title}</h1>
      {@render head?.()}
    </header>
  {:else if title}
    <header class="large-title"><h1 class="clip">{title}</h1>{@render head?.()}</header>
  {/if}
  {#if error}<p class="empty">{W.failed(error)}</p>
  {:else if !ready}<p class="loading">{W.loading}</p>
  {:else}{@render children()}{/if}
</div>

<style>
  /* The page scrolls under the chrome; this keeps its content clear of it at rest. */
  .page {
    padding: var(--top-h) var(--gutter) calc(var(--bar-h) + var(--gap-section));
  }
  .large-title h1 {
    flex: 1;
    font-size: 31px;
    line-height: 40px;
  }
  .back-title {
    display: flex;
    align-items: center;
    margin: 0 0 6px -16px;
  }
  .back-title .glyph :global(.icon) {
    width: 22px;
    height: 22px;
  }
  .back-title h1 {
    flex: 1;
    font-size: 24px;
    line-height: 32px;
  }
  @media (max-width: 719px) {
    .page {
      padding-bottom: calc(var(--bar-h) + var(--gap-section) + env(safe-area-inset-bottom));
    }
  }
</style>
