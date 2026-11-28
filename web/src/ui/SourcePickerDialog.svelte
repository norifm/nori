<script lang="ts">
  // Sheet picking another saved profile to merge into a profile, or signing in to a new one.
  import type { Profile } from "../state/types";
  import { W } from "../words";
  import Icon from "./Icon.svelte";
  import ProfileMark from "./ProfileMark.svelte";

  let { title, offered, pick, signIn, close }: { title: string; offered: Profile[]; pick: (id: string) => void; signIn: () => void; close: () => void } = $props();
  let dialog = $state<HTMLDialogElement>();
  $effect(() => {
    dialog?.showModal();
  });

  const labelOf = (p: Profile) => p.label || (p.kind === "local" ? W.local : W.subsonic);
  const detailOf = (p: Profile) => p.user || (p.kind === "local" ? W.local : p.host || W.subsonic);
</script>

<dialog class="menu card" id="add-source" bind:this={dialog} onclose={close}>
  <div class="body">
    <h2>{title}</h2>
    <div class="group">
      {#each offered as o (o.id)}
        <button class="group-row" data-offered={o.id} onclick={() => pick(o.id)}>
          <ProfileMark icon={o.icon} letter={o.letter} kind={o.kind} />
          <span class="words"><span>{labelOf(o)}</span><span class="sub">{detailOf(o)}</span></span>
        </button>
      {/each}
      <button class="group-row" id="source-sign-in" onclick={signIn}>
        <Icon name="plus" />
        <span class="words">{W.sourceSignIn}</span>
      </button>
    </div>
    <div class="end"><button type="button" class="text-action" onclick={close}>{W.cancel}</button></div>
  </div>
</dialog>
