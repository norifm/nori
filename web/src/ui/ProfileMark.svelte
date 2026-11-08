<script lang="ts">
  // A profile's mark on a circle, as its icon says: its letter, its provider's mark (a server's glyph: every
  // profile here is an OpenSubsonic server's), or one of Iconoir's faces. `side` its width, px.
  import type { ProfileIcon } from "../state/types";
  import Icon from "./Icon.svelte";

  let { icon, letter, side = 40 }: { icon: ProfileIcon; letter: string; side?: number } = $props();
  const glyphs: Record<ProfileIcon, string> = {
    Letter: "",
    Provider: "server",
    Sing: "emoji-sing-right",
    Surprise: "emoji-surprise-alt",
    Talking: "emoji-talking-happy",
    LookUp: "emoji-look-up",
    Satisfied: "emoji-satisfied",
    Think: "emoji-think-right",
  };
</script>

<span class="mark" style:--side="{side}px" data-icon={icon}>{#if icon === "Letter"}{letter}{:else}<Icon name={glyphs[icon]} />{/if}</span>

<style>
  .mark {
    display: inline-grid;
    place-items: center;
    flex: none;
    width: var(--side);
    height: var(--side);
    border-radius: 50%;
    background: color-mix(in srgb, var(--ink) 10%, var(--surface));
    color: var(--ink);
    font-size: calc(var(--side) * 0.42);
    line-height: 1;
    font-weight: 600;
  }
  .mark :global(.icon) {
    width: calc(var(--side) * 0.6);
    height: calc(var(--side) * 0.6);
  }
</style>
