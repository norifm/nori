<script lang="ts">
  // The volume of the device playing: a speaker glyph and a slider. `ends`: the quiet speaker before the
  // slider and the loud one after it, as the full player's; `mute`: the speaker a button that mutes, and
  // again brings back the level it had, as the player bar's.
  import { player } from "../state/player.svelte";
  import { W } from "../words";
  import Icon from "./Icon.svelte";

  let { ends = false, mute = false }: { ends?: boolean; mute?: boolean } = $props();
  const level = $derived(player.now?.volume ?? 1);
  const glyph = $derived(level === 0 ? "sound-off" : level < 0.5 ? "sound-low" : "sound-high");
  /** The level before a mute. */
  let before = 1;
  const set = (v: number) => void player.control("volume", v);
  function toggle() {
    if (level > 0) {
      before = level;
      set(0);
    } else set(before || 1);
  }
</script>

<div class="vol" class:mute>
  {#if mute}
    <button class="glyph quiet" aria-label={W.volume} title={W.volume} onclick={toggle}><Icon name={glyph} /></button>
  {:else}
    <Icon name={ends ? "sound-low" : glyph} />
  {/if}
  <input class="range" type="range" min="0" max="1" step="0.01" value={level} style:--v="{level * 100}%" aria-label={W.volume} oninput={(e) => set(Number(e.currentTarget.value))} />
  {#if ends}<Icon name="sound-high" />{/if}
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
  .mute {
    width: auto;
    gap: 4px;
  }
  .mute input {
    width: 84px;
    flex: none;
  }
  input {
    flex: 1;
    min-width: 0;
  }
</style>
