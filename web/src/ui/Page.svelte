<script lang="ts">
  // A page: its big title (with a way back where it is not a top page, and its own controls beside),
  // then what it shows; while its read is out, a quiet word; if it failed, why. `narrow`: one column
  // centered under a centered title on a wide window, as the desktop's settings.
  import type { Snippet } from "svelte";
  import { W } from "../words";
  import Icon from "./Icon.svelte";

  let { title = "", back = false, narrow = false, error, ready = true, head, children }: { title?: string; back?: boolean; narrow?: boolean; error?: string; ready?: boolean; head?: Snippet; children: Snippet } = $props();
</script>

<div class="page" class:narrow>
  {#if back}<button class="glyph back" aria-label={W.back} title={W.back} onclick={() => history.back()}><Icon name="arrow-left" /></button>{/if}
  {#if title}
    <header class="page-head">
      <h1>{title}</h1>
      {@render head?.()}
    </header>
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
  .back {
    margin: 0 0 2px -12px;
  }
  .back :global(.icon) {
    width: 22px;
    height: 22px;
  }
  .page-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    margin: 6px 0 18px;
    flex-wrap: wrap;
  }
  h1 {
    font-size: 34px;
    line-height: 42px;
  }
  @media (min-width: 720px) {
    .narrow {
      max-width: calc(720px + 2 * var(--gutter));
      margin: 0 auto;
    }
    .narrow .page-head {
      justify-content: center;
    }
    .narrow h1 {
      text-align: center;
    }
  }
  @media (max-width: 719px) {
    .page {
      padding-bottom: calc(var(--bar-h) + var(--gap-section) + env(safe-area-inset-bottom));
    }
    h1 {
      font-size: 31px;
      line-height: 40px;
    }
  }
  .page :global(h2.section) {
    font-size: 19px;
    line-height: 24px;
    letter-spacing: -0.2px;
    margin: 30px 0 14px;
  }
</style>
