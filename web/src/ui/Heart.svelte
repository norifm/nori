<script lang="ts">
  // A heart on a song, album or artist (`what`), drawn as `on`, that turns it the other way.
  import { library } from "../state/library.svelte";
  import { player } from "../state/player.svelte";
  import { W } from "../words";
  import Icon from "./Icon.svelte";

  let { what, id, on }: { what: "song" | "album" | "artist"; id: string; on: boolean } = $props();
</script>

<button
  class="round heart"
  class:on
  title={on ? W.unfavorite : W.favorite}
  aria-label={on ? W.unfavorite : W.favorite}
  aria-pressed={on}
  onclick={(e) => {
    e.stopPropagation();
    void library.change({ kind: "star", what, id, on: !on }).then(() => player.refresh());
  }}><Icon name={on ? "heart" : "heart-line"} /></button
>

<style>
  .heart {
    color: var(--dim);
  }
  .heart.on {
    color: var(--accent);
  }
</style>
