<script lang="ts">
  // Play On: this browser and the account's other devices, the one playing ticked; a click moves the
  // music there (through the server's relay: a browser cannot reach devices on its network itself).
  // Under them the jam, as in Android's devices sheet.
  import { remote } from "../state/remote.svelte";
  import { W } from "../words";
  import Icon from "./Icon.svelte";
  import JamHost from "./JamHost.svelte";

  const d = $derived(remote.devices);
  const kindIcon: Record<string, string> = { Phone: "phone", Desktop: "laptop", Terminal: "terminal", Guest: "artist" };
  $effect(() => void remote.read());
</script>

<div class="devices" id="devices">
  {#if !d.on}
    <p class="sub">{W.devicesOff}</p>
  {:else}
    <p class="sub lead">{W.devicesLead}</p>
    <button class="device" class:on={!d.active} aria-pressed={!d.active} data-device="" onclick={() => void remote.pick("")}>
      <Icon name="laptop" /><span class="name">{W.thisBrowser}</span>{#if !d.active}<Icon name="check" />{/if}
    </button>
    {#each d.list as x (x.id)}
      <button class="device" class:on={d.active === x.id} aria-pressed={d.active === x.id} data-device={x.id} onclick={() => void remote.pick(x.id)}>
        <Icon name={kindIcon[x.kind] ?? "devices"} /><span class="name clip">{x.name}</span>
        {#if d.active === x.id}<Icon name="check" />{:else if x.playing}<span class="sub">{W.devicePlaying}</span>{/if}
      </button>
    {:else}
      <p class="sub">{W.devicesNone}</p>
    {/each}
    <JamHost />
  {/if}
</div>

<style>
  .devices {
    display: flex;
    flex-direction: column;
    gap: 2px;
    overflow-y: auto;
  }
  .lead {
    margin: 0 4px 10px;
  }
  .device {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 10px;
    border-radius: 12px;
    text-align: left;
    font-weight: 500;
  }
  .device:hover {
    background: var(--hover);
  }
  .device.on :global(.icon:first-child),
  .device.on .name {
    color: var(--accent);
  }
  .device.on .name {
    font-weight: 600;
  }
  .device :global(.icon) {
    width: 22px;
    height: 22px;
  }
  .name {
    flex: 1;
  }
  .sub {
    padding: 4px;
  }
</style>
