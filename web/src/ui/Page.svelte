<script lang="ts">
  // A page: its big title (with a way back where it is not a top page, and its own controls beside),
  // then what it shows; while its read is out, its `sketch` (Reads: a `sketch::Reading` by name) or else a
  // quiet word; if it failed, why. `narrow`: on a wide window, the page stands in one column centered in
  // it, the top bar's and the player's, edge to edge with them.
  import type { Snippet } from "svelte";
  import { ask } from "../rpc";
  import { W } from "../words";
  import Icon from "./Icon.svelte";
  import Reads from "./Reads.svelte";
  import { PLACES, sectionPage } from "./TopPill.svelte";

  let { title = "", back = false, narrow = false, places = false, error, ready = true, sketch, head, children }: { title?: string; back?: boolean; narrow?: boolean; places?: boolean; error?: string; ready?: boolean; sketch?: string; head?: Snippet; children: Snippet } = $props();
  /** `places`: a library page, with the places the profile keeps (History) beside its title, a pill each
   *  with its glyph and its name, as Android's. */
  let kept = $state<string[]>([]);
  $effect(() => {
    if (places) void ask<string>("library_sections").then((s) => (kept = (JSON.parse(s) as string[]).filter((x) => PLACES.includes(x))));
  });
</script>

<div class="page" class:narrow class:has-back={back} class:titled={!!title}>
  {#if back}<button class="glyph back" aria-label={W.back} title={W.back} onclick={() => history.back()}><Icon name="arrow-left" /></button>{/if}
  {#if title}
    <header class="page-head">
      <h1>{title}</h1>
      {#if kept.length}
        <span class="places">
          {#each kept as p (p)}<a class="chip edged place" href={sectionPage[p][0]}><Icon name={sectionPage[p][1]} />{W.librarySections[p]}</a>{/each}
        </span>
      {/if}
      {@render head?.()}
    </header>
  {/if}
  {#if error}<p class="empty">{W.failed(error)}</p>
  {:else}
    {#if sketch}<Reads {ready} reading={sketch} />{/if}
    {#if ready}{@render children()}{:else if !sketch}<p class="loading">{W.loading}</p>{/if}
  {/if}
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
    gap: 8px;
    margin-left: auto;
  }
  .place {
    gap: 6px;
    color: var(--ink);
    font-size: 13px;
  }
  .place:hover {
    background: var(--hover);
  }
  .place :global(.icon) {
    width: 16px;
    height: 16px;
  }
  @media (min-width: 720px) {
    .narrow {
      width: var(--pill-w);
      margin-inline: auto;
      padding-inline: 0;
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
  /* The desktop's frame on a wide window: the title 84 px down, the way back a disc beside it (or alone
     above what the page shows, which starts 140 px down). */
  @media (min-width: 720px) {
    .page {
      position: relative;
    }
    .page.has-back:not(.titled) {
      padding-top: 140px;
    }
    .back {
      position: absolute;
      top: 87px;
      left: var(--gutter);
      width: 40px;
      height: 40px;
      margin: 0;
      border: 1px solid var(--outline);
      background: color-mix(in srgb, var(--page) 70%, transparent);
    }
    .narrow .back {
      left: 0;
    }
    .back :global(.icon) {
      width: 20px;
      height: 20px;
    }
    .page-head {
      margin: -1px 0 17px;
    }
    .has-back .page-head {
      padding-left: 52px;
    }
    /* A page of one column (Settings) stands its title 8 px higher, as the desktop's. */
    .narrow:not(.has-back) .page-head {
      margin-top: -9px;
    }
    h1 {
      line-height: 52px;
      letter-spacing: -0.5px;
    }
    .page :global(h2.section) {
      font-size: 21px;
      line-height: 32px;
      margin: 26px 0 14px;
    }
  }
</style>
