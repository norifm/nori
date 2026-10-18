<script lang="ts">
  // A favorite on a song, album or artist (`what`), drawn as `on`: the seal. Not liked it is the outline,
  // upright and quiet; liked it is filled with the accent, tilted, and stamped on when the click made it
  // so. A seal that changes by itself (another song) is drawn, not stamped.
  import { library } from "../state/library.svelte";
  import { player } from "../state/player.svelte";
  import { W } from "../words";
  import Icon from "./Icon.svelte";

  let { what, id, on }: { what: "song" | "album" | "artist"; id: string; on: boolean } = $props();
  let stamp = $state(false);

  // A like taken off ends the stamp.
  $effect(() => {
    if (!on) stamp = false;
  });
</script>

<button
  class="round seal"
  class:on
  class:stamp
  title={on ? W.unfavorite : W.favorite}
  aria-label={on ? W.unfavorite : W.favorite}
  aria-pressed={on}
  onclick={(e) => {
    e.stopPropagation();
    stamp = !on;
    void library.change({ kind: "star", what, id, on: !on }).then(() => player.refresh());
  }}><Icon name={on ? "seal" : "seal-line"} /></button
>

<style>
  .seal {
    color: var(--ink2);
  }
  .seal.on {
    color: var(--accent);
  }
  .seal.on :global(.icon) {
    transform: rotate(-5deg);
  }
  .seal.stamp :global(.icon) {
    animation: stamp 420ms cubic-bezier(0.3, 1.6, 0.5, 1);
  }
  @keyframes stamp {
    from {
      transform: rotate(-5deg) scale(1.5);
    }
    to {
      transform: rotate(-5deg) scale(1);
    }
  }
</style>
