<script lang="ts">
  // An album's, artist's or playlist's card: its cover with the card's corners (an artist's round, its
  // name under it in the middle), its name and a quiet line under it.
  import Cover from "./Cover.svelte";

  let { href, cover, title, sub = "", round = false, fallback = "music-note" }: { href: string; cover?: string; title: string; sub?: string; round?: boolean; fallback?: string } = $props();
</script>

<a class="card" class:round {href}>
  <Cover src={cover} {round} radius="var(--r-card)" {fallback} />
  <span class="title clip">{title}</span>
  {#if sub}<span class="sub clip">{sub}</span>{/if}
</a>

<style>
  .card {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .title {
    margin-top: 8px;
    font-size: 14px;
    line-height: 19px;
    font-weight: 500;
  }
  .sub {
    font-size: 12.5px;
    line-height: 16px;
  }
  .round {
    text-align: center;
  }
  .round .title {
    margin-top: 7px;
    font-weight: 400;
  }
</style>
