<script lang="ts">
  // Names on the page's paper, each opening what it names, as Android's rows: its count and an arrow at its
  // end, raised under the pointer (Genres, Decades). The words start at the column's edge; the hover reaches
  // 8 px past it.
  import Icon from "./Icon.svelte";

  let { lines }: { lines: { key: string; href: string; name: string; count: number }[] } = $props();
</script>

<ul class="lines">
  {#each lines as l (l.key)}
    <li><a href={l.href}><span class="name clip">{l.name}</span><span class="count">{l.count}</span><Icon name="nav-arrow-right" /></a></li>
  {/each}
</ul>

<style>
  .lines {
    list-style: none;
    margin: 0 -8px;
    padding: 0;
  }
  a {
    display: flex;
    align-items: center;
    gap: 12px;
    height: 48px;
    padding: 0 8px;
    border-radius: 10px;
    color: var(--ink);
    text-decoration: none;
  }
  a:hover {
    background: var(--hover);
  }
  .name {
    flex: 1;
    min-width: 0;
    font-size: 15px;
  }
  .count {
    color: var(--ink2);
    font-size: 13px;
    font-variant-numeric: tabular-nums;
  }
  a :global(.icon) {
    width: 18px;
    height: 18px;
    color: var(--ink2);
  }
</style>
