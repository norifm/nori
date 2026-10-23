<script lang="ts">
  // Play On: this browser and the account's other devices, each with its kind's glyph, the one playing
  // lit and ticked; a click moves the music there (through the server's relay: a browser cannot reach
  // devices on its network itself). Under them the jam, as in Android's devices sheet.
  import { remote } from "../state/remote.svelte";
  import { W } from "../words";
  import Icon from "./Icon.svelte";
  import JamHost from "./JamHost.svelte";
  import { kindGlyph } from "./OutputButton.svelte";

  const d = $derived(remote.devices);
  $effect(() => void remote.read());
</script>

<div class="devices" id="devices">
  {#if !d.on}
    <p class="sub">{W.devicesOff}</p>
  {:else}
    <div class="group">
      <button class="group-row device" class:on={!d.active} aria-pressed={!d.active} data-device="" onclick={() => void remote.pick("")}>
        <Icon name="computer" /><span class="words">{W.thisBrowser}</span>{#if !d.active}<Icon name="check" />{/if}
      </button>
      {#each d.list as x (x.id)}
        <button class="group-row device" class:on={d.active === x.id} aria-pressed={d.active === x.id} data-device={x.id} onclick={() => void remote.pick(x.id)}>
          <Icon name={kindGlyph[x.kind] ?? "computer"} /><span class="words">{x.name}</span>
          {#if d.active === x.id}<Icon name="check" />{:else if x.playing}<span class="value">{W.devicePlaying}</span>{/if}
        </button>
      {/each}
    </div>
    {#if !d.list.length}<p class="sub">{W.devicesNone}</p>{/if}
    <JamHost />
  {/if}
</div>

<style>
  .devices {
    display: flex;
    flex-direction: column;
    gap: 10px;
    overflow-y: auto;
  }
  .device.on > :global(.icon:first-child) {
    color: var(--accent);
  }
  .sub {
    margin: 0;
    padding: 4px;
  }
</style>
