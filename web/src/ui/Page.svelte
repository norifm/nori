<script lang="ts">
  // A page: its big title (with a way back where it is not a top page, and its own controls beside),
  // then what it shows; while its read is out, a quiet word; if it failed, why.
  import type { Snippet } from "svelte";
  import { W } from "../words";
  import Icon from "./Icon.svelte";

  let { title = "", back = false, error, ready = true, head, children }: { title?: string; back?: boolean; error?: string; ready?: boolean; head?: Snippet; children: Snippet } = $props();
</script>

<div class="page">
  {#if back}<button class="round back" aria-label={W.back} title={W.back} onclick={() => history.back()}><Icon name="back" /></button>{/if}
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
  .page {
    padding: 28px var(--page-pad) 48px;
    max-width: 1400px;
  }
  .back {
    margin: -8px 0 6px -8px;
    color: var(--accent);
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
    margin-bottom: 22px;
    flex-wrap: wrap;
  }
  h1 {
    font-size: 30px;
    font-weight: 700;
  }
  @media (max-width: 899px) {
    .page {
      padding-top: calc(16px + env(safe-area-inset-top));
    }
    h1 {
      font-size: 32px;
    }
  }
  .page :global(h2.section) {
    font-size: 21px;
    font-weight: 700;
    margin: 30px 0 14px;
  }
</style>
