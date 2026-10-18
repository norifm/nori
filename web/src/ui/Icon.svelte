<script lang="ts">
  // An icon from the sprite in index.html.
  let { name }: { name: string } = $props();
</script>

<svg class="icon" aria-hidden="true"><use href="#i-{name}" /></svg>
