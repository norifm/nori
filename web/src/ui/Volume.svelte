<script lang="ts">
  // The volume of the device playing: a speaker glyph and a slider.
  import { player } from "../state/player.svelte";
  import { W } from "../words";
  import Icon from "./Icon.svelte";

  const level = $derived(player.now?.volume ?? 1);
</script>

<div class="vol">
  <Icon name={level === 0 ? "sound-off" : level < 0.5 ? "sound-low" : "sound-high"} />
  <input
    class="range"
    type="range"
    min="0"
    max="1"
    step="0.01"
    value={level}
    style:--v="{level * 100}%"
    aria-label={W.volume}
    oninput={(e) => void player.control("volume", Number(e.currentTarget.value))}
  />
</div>

<style>
  .vol {
    display: flex;
    align-items: center;
    gap: 7px;
    width: 110px;
    color: var(--ink2);
  }
  .vol :global(.icon) {
    width: 20px;
    height: 20px;
  }
  input {
    flex: 1;
    min-width: 0;
  }
</style>
