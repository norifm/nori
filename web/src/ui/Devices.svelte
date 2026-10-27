<script lang="ts">
  // The devices, as the desktop's panel: this browser and the account's other devices, each with its
  // kind's glyph in a disc, the one playing lit and ticked; a click moves the music there (through the server's relay: a browser cannot reach
  // devices on its network itself). Under them the jam, as in Android's devices sheet.
  import { remote } from "../state/remote.svelte";
  import { W } from "../words";
  import Icon from "./Icon.svelte";
  import JamHost from "./JamHost.svelte";
  import PanelRow from "./PanelRow.svelte";
  import { kindGlyph } from "./OutputButton.svelte";

  const d = $derived(remote.devices);
  $effect(() => void remote.read());
</script>

<div class="devices" id="devices">
  {#if !d.on}
    <p class="sub">{W.devicesOff}</p>
  {:else}
    <div class="rows">
      <PanelRow icon="computer" title={W.thisBrowser} lit={!d.active} data-device="" onclick={() => void remote.pick("")}>
        {#snippet end()}{#if !d.active}<Icon name="check" />{/if}{/snippet}
      </PanelRow>
      {#each d.list as x (x.id)}
        <PanelRow icon={kindGlyph[x.kind] ?? "computer"} title={x.name} sub={x.playing && d.active !== x.id ? W.devicePlaying : ""} lit={d.active === x.id} data-device={x.id} onclick={() => void remote.pick(x.id)}>
          {#snippet end()}{#if d.active === x.id}<Icon name="check" />{/if}{/snippet}
        </PanelRow>
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
    gap: 4px;
    overflow-y: auto;
  }
  .rows {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .sub {
    margin: 0;
    padding: 4px;
  }
</style>
