<script lang="ts">
  // A favorite on a song, album or artist (`what`), drawn as `on`: the seal 好. Not a favorite it is its
  // outline in the quiet ink; a favorite it is the stamped seal in the accent, stamped down when the
  // click made it one. A seal that changes by itself (another song) is drawn, not stamped. Its glyph is the
  // size of the glyphs beside it (its row sets them), so its line is theirs; its target is 48 px.
  import { library } from "../state/library.svelte";
  import { player } from "../state/player.svelte";
  import { W } from "../words";
  import Icon from "./Icon.svelte";

  let { what, id, on }: { what: "song" | "album" | "artist"; id: string; on: boolean } = $props();
  let stamp = $state(false);

  // A favorite taken off ends the stamp.
  $effect(() => {
    if (!on) stamp = false;
  });
</script>

<button
  class="glyph seal"
  class:on
  class:stamp
  title={on ? W.removeFromFavorites : W.addToFavorites}
  aria-label={on ? W.removeFromFavorites : W.addToFavorites}
  aria-pressed={on}
  onclick={(e) => {
    e.stopPropagation();
    stamp = !on;
    void library.change({ kind: "star", what, id, on: !on }).then(() => player.refresh());
  }}><Icon name={on ? "sealed" : "seal"} /></button
>

<style>
  .seal {
    color: var(--ink2);
  }
  .seal.on {
    color: var(--accent);
  }
  .seal.stamp :global(.icon) {
    animation: stamp 420ms cubic-bezier(0.3, 1.6, 0.5, 1);
  }
  @keyframes stamp {
    from {
      transform: scale(1.5);
    }
  }
</style>
