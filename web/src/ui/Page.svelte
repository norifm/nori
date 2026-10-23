<script lang="ts">
  // A page: its big title (with a way back where it is not a top page, and its own controls beside),
  // then what it shows; while its read is out, a quiet word; if it failed, why.
  import type { Snippet } from "svelte";
  import { W } from "../words";
  import Icon from "./Icon.svelte";

  let { title = "", back = false, error, ready = true, head, children }: { title?: string; back?: boolean; error?: string; ready?: boolean; head?: Snippet; children: Snippet } = $props();
</script>

<div class="page">
  {#if back}<button class="round back" aria-label={W.back} title={W.back} onclick={() => history.back()}><Icon name="arrow-left" /></button>{/if}
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
  /* The page scrolls under the pills; this keeps its content clear of both at rest. */
  .page {
    padding: var(--top-h) var(--page-pad) calc(var(--bar-h) + 24px);
  }
  .back {
    margin: 0 0 6px -8px;
    color: var(--ink2);
  }
  .back :global(.icon) {
    width: 26px;
    height: 26px;
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
    font-weight: 700;
    letter-spacing: -0.3px;
  }
  @media (max-width: 719px) {
    .page {
      padding-bottom: calc(var(--bar-h) + 24px + env(safe-area-inset-bottom));
    }
  }
  .page :global(h2.section) {
    font-size: 20px;
    font-weight: 700;
    letter-spacing: -0.2px;
    margin: 30px 0 14px;
  }
</style>
