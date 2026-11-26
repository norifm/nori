<script lang="ts">
  // An album's, artist's or playlist's card: its cover with the card's corners (an artist's round, its
  // name under it in the middle), its name and a quiet line under it. A right click opens its menu (with
  // Delete for a playlist the account may change, `editable`). A provider's (`remote`, not in the library
  // yet) has the cloud before its line, as Android's. Opened, it tells `onopen`.
  import { itemMark } from "../state/menus";
  import type { Whole } from "../state/types";
  import Cover from "./Cover.svelte";
  import Icon from "./Icon.svelte";

  let { href, kind, id, cover, title, sub = "", round = false, fallback = "music-note", editable = false, remote = false, onopen }: { href: string; kind: Whole; id: string; cover?: string; title: string; sub?: string; round?: boolean; fallback?: string; editable?: boolean; remote?: boolean; onopen?: () => void } = $props();
</script>

<a class="card" class:round {href} onclick={onopen} {...itemMark(kind, id, { title, line: sub, cover }, editable)}>
  <Cover src={cover} {round} radius="var(--r-card)" {fallback} />
  <span class="title" class:clip={!round}>{title}</span>
  {#if sub || remote}<span class="sub clip">{#if remote}<Icon name="cloud-download" />{/if}{sub}</span>{/if}
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
  .sub :global(.icon) {
    width: 13px;
    height: 13px;
    margin-right: 4px;
    vertical-align: -2px;
  }
  .round {
    text-align: center;
  }
  /* Two lines: names that start alike ("Aphex Twin", "Aphex Twin & …") are told apart by how they end. */
  .round .title {
    margin-top: 7px;
    font-weight: 400;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
</style>
