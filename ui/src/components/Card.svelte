<script lang="ts">
  import type { Snippet } from 'svelte';

  interface Props {
    children?: Snippet;
  }

  let { children }: Props = $props();
</script>

<div class="card">
  {@render children?.()}
</div>

<style>
  .card {
    background: var(--card);
    border: 1px solid var(--stroke);
    border-radius: 6px;
  }
</style>
