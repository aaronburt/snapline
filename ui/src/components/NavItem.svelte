<script lang="ts">
  import type { Snippet } from 'svelte';
  import Icon from './Icon.svelte';
  import type { IconName } from '../lib/icons';

  interface Props {
    label: string;
    icon: IconName;
    count?: string;
    active?: boolean;
    height?: 38 | 40;
    href?: string;
    onclick?: (event: MouseEvent) => void;
  }

  let {
    label,
    icon,
    count,
    active = false,
    height = 38,
    href,
    onclick,
  }: Props = $props();

  const barTop = $derived(height === 40 ? 12 : 11);
</script>

{#snippet content()}
  {#if active}
    <span class="bar" aria-hidden="true" style:top="{barTop}px"></span>
  {/if}
  <Icon name={icon} size={16} />
  <span class="label">{label}</span>
  {#if count}
    <span class="count">{count}</span>
  {/if}
{/snippet}

{#if href}
  <a
    class="hv navitem"
    class:active
    {href}
    style:height="{height}px"
    aria-current={active ? 'page' : undefined}
  >
    {@render content()}
  </a>
{:else}
  <button
    type="button"
    class="hv navitem"
    class:active
    style:height="{height}px"
    aria-current={active ? 'page' : undefined}
    {onclick}
  >
    {@render content()}
  </button>
{/if}

<style>
  .navitem {
    position: relative;
    height: 38px;
    width: 100%;
    border: none;
    border-radius: 4px;
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 0 12px 0 16px;
    background: transparent;
    color: var(--text);
    font-size: 13px;
    text-align: left;
  }

  .navitem.active {
    background: var(--subtle);
  }

  .bar {
    position: absolute;
    left: 4px;
    width: 3px;
    height: 16px;
    border-radius: 2px;
    background: var(--accent);
  }

  .label {
    flex: 1;
    min-width: 0;
  }

  .count {
    font-size: 12px;
    color: var(--text3);
  }
</style>
