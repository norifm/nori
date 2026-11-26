<script lang="ts">
  // A page: its big title (with a way back where it is not a top page, and its own controls beside),
  // then what it shows; while its read is out, a quiet word; if it failed, why. `narrow`: one column at a
  // reading width on a wide window, from the gutter as every page's title is, as the desktop's settings.
  import type { Snippet } from "svelte";
  import { ask } from "../rpc";
  import { W } from "../words";
  import Icon from "./Icon.svelte";
  import { PLACES, sectionPage } from "./TopPill.svelte";

  let { title = "", back = false, narrow = false, places = false, error, ready = true, head, children }: { title?: string; back?: boolean; narrow?: boolean; places?: boolean; error?: string; ready?: boolean; head?: Snippet; children: Snippet } = $props();
  /** `places`: a library page, with the places the profile keeps (History) beside its title. */
  let kept = $state<string[]>([]);
  $effect(() => {
    if (places) void ask<string>("library_sections").then((s) => (kept = (JSON.parse(s) as string[]).filter((x) => PLACES.includes(x))));
  });
</script>

<div class="page" class:narrow>
  {#if back}<button class="glyph back" aria-label={W.back} title={W.back} onclick={() => history.back()}><Icon name="arrow-left" /></button>{/if}
  {#if title}
    <header class="page-head">
      <h1>{title}</h1>
      {#if kept.length}
        <span class="places">
          {#each kept as p (p)}<a class="glyph quiet" href={sectionPage[p][0]} aria-label={W.librarySections[p]} title={W.librarySections[p]}><Icon name={sectionPage[p][1]} /></a>{/each}
        </span>
      {/if}
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
  .places {
    display: flex;
    margin-left: auto;
  }
  @media (min-width: 720px) {
    .narrow {
      max-width: calc(720px + 2 * var(--gutter));
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
