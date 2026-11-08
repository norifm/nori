<script lang="ts">
  // A profile's icon picked: every icon its circle can show, as the circle would draw it, four to a line in
  // the core's order, the one shown ringed in the accent. A click picks one and closes the card.
  import type { Profile, ProfileIcon } from "../state/types";
  import { W } from "../words";
  import ProfileMark from "./ProfileMark.svelte";

  let { profile, pick, close }: { profile: Profile; pick: (icon: ProfileIcon) => void; close: () => void } = $props();
  let dialog = $state<HTMLDialogElement>();
  $effect(() => dialog?.showModal());
</script>

<dialog class="card" id="profile-icons" bind:this={dialog} onclose={close}>
  <h2>{W.profileIcon}</h2>
  <div class="icons" role="radiogroup" aria-label={W.profileIcon}>
    {#each profile.icons as icon (icon)}
      {@const on = icon === profile.icon}
      <button
        class="pick"
        class:on
        role="radio"
        aria-checked={on}
        aria-label={W.profileIcons[icon] ?? icon}
        title={W.profileIcons[icon] ?? icon}
        data-icon={icon}
        onclick={() => {
          close();
          if (!on) pick(icon);
        }}><ProfileMark {icon} letter={profile.letter} side={46} /></button
      >
    {/each}
  </div>
  <div class="end"><button type="button" class="text-action" onclick={close}>{W.cancel}</button></div>
</dialog>

<style>
  .icons {
    display: grid;
    grid-template-columns: repeat(4, 56px);
    justify-content: space-between;
    row-gap: 14px;
    padding: 6px 0;
  }
  /* The ring round the mark: a hairline, or the accent on the one shown. */
  .pick {
    display: grid;
    place-items: center;
    width: 56px;
    height: 56px;
    border-radius: 50%;
    box-shadow: inset 0 0 0 1px var(--hair);
  }
  .pick:hover {
    background: var(--hover);
  }
  .pick.on {
    box-shadow: inset 0 0 0 2px var(--accent);
  }
</style>
