<script lang="ts">
  import type { Snippet } from 'svelte';
  import { settingsError } from './settings';

  interface Props {
    title: string;
    intro?: string;
    children?: Snippet;
  }

  let { title, intro = '', children }: Props = $props();
</script>

<div class="page">
  <header>
    <h1>{title}</h1>
    {#if intro}
      <p class="intro">{intro}</p>
    {/if}
  </header>
  {#if $settingsError}
    <p class="notice" role="status">{$settingsError}</p>
  {/if}
  {@render children?.()}
</div>

<style>
  .page {
    display: flex;
    flex-direction: column;
    gap: 28px;
  }

  header {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  h1 {
    margin: 0;
    font-size: 28px;
    line-height: 36px;
    font-weight: 600;
  }

  .intro {
    margin: 0;
    font-size: 12px;
    color: var(--text3);
  }

  .notice {
    margin: 0;
    padding: 10px 12px;
    border: 1px solid var(--err);
    border-radius: 4px;
    color: var(--err);
    font-size: 12px;
  }
</style>
