<script lang="ts">
  // The song playing's glyph, as the desktop's and Android's: four bars in the ink around them, at speeds
  // that never fall into step, moving while the music does and standing at rest while it waits or is paused.
  // `w` and `h` its size, px.
  let { moving, w = 17, h = 16 }: { moving: boolean; w?: number; h?: number } = $props();
</script>

<span class="bars" class:moving style:width="{w}px" style:height="{h}px" aria-hidden="true"><i></i><i></i><i></i><i></i></span>

<style>
  /* Each bar a seventh of the width, a gap of one between; at rest at the desktop's heights. */
  .bars {
    display: inline-flex;
    align-items: flex-end;
    justify-content: space-between;
    flex: none;
  }
  i {
    width: calc(100% / 7);
    height: 100%;
    border-radius: 99px;
    background: currentColor;
    transform-origin: bottom;
    transform: scaleY(var(--rest));
  }
  i:nth-child(1) {
    --rest: 0.53;
    --period: 1.23s;
  }
  i:nth-child(2) {
    --rest: 0.86;
    --period: 0.98s;
  }
  i:nth-child(3) {
    --rest: 0.64;
    --period: 0.82s;
  }
  i:nth-child(4) {
    --rest: 0.75;
    --period: 0.7s;
  }
  .moving i {
    animation: rise calc(var(--period) / 2) ease-in-out infinite alternate;
  }
  @keyframes rise {
    from {
      transform: scaleY(0.28);
    }
    to {
      transform: scaleY(1);
    }
  }
</style>
