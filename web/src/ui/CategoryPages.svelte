<script lang="ts">
  // What the repo offers on a page per category, as Android's Get plugins: swiped between, or picked from
  // the chips above them. Offers of one category alone are one page with no chips. `order`: the
  // categories in the order their pages go.
  import type { Snippet } from "svelte";
  import type { Offer } from "../plugins/types";
  import { W } from "../words";

  let { offers, order, page }: { offers: Offer[]; order: string[]; page: Snippet<[Offer[]]> } = $props();

  const shown = $derived(order.filter((c) => offers.some((o) => o.category === c)));
  let pages = $state<HTMLElement>();
  let at = $state(0);
  const lit = $derived(Math.min(at, shown.length - 1));
  // The pages' smooth scroll is CSS's, which nori's Motion setting stills.
  const pick = (i: number) => pages?.scrollTo({ left: i * pages.clientWidth });
  const scrolled = () => pages && (at = Math.round(pages.scrollLeft / pages.clientWidth));
</script>

{#if shown.length < 2}
  {@render page(offers)}
{:else}
  <div class="chips" role="tablist">
    {#each shown as c, i (c)}
      <button class="chip" class:on={lit === i} role="tab" aria-selected={lit === i} data-category={c} onclick={() => pick(i)}>{W.pluginCategory(c)}</button>
    {/each}
  </div>
  <div class="pages" bind:this={pages} onscroll={scrolled}>
    {#each shown as c (c)}
      <div class="page" data-category-page={c}>{@render page(offers.filter((o) => o.category === c))}</div>
    {/each}
  </div>
{/if}

<style>
  .chips {
    margin-bottom: 8px;
  }
  .pages {
    display: flex;
    align-items: flex-start;
    overflow-x: auto;
    scroll-snap-type: x mandatory;
    scroll-behavior: smooth;
    scrollbar-width: none;
  }
  .pages::-webkit-scrollbar {
    display: none;
  }
  .page {
    flex: 0 0 100%;
    min-width: 0;
    scroll-snap-align: start;
  }
</style>
