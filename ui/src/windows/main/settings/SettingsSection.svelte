<script lang="ts">
  import type { Snippet } from 'svelte';

  interface Props {
    title: string;
    description?: string;
    children?: Snippet;
  }

  let { title, description = '', children }: Props = $props();
</script>

<section aria-label={title}>
  <h2>{title}</h2>
  {#if description}
    <p class="description">{description}</p>
  {/if}
  <div class="rows">
    {@render children?.()}
  </div>
</section>

<style>
  section {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  h2 {
    margin: 0 0 4px;
    font-size: 14px;
    font-weight: 600;
  }

  .description {
    margin: 0 0 6px;
    font-size: 12px;
    color: var(--text3);
  }

  .rows {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
</style>
