<script lang="ts">
  import type { Snippet } from 'svelte';
  import Icon from './Icon.svelte';
  import type { IconName } from '../lib/icons';

  interface Props {
    variant?: 'accent' | 'secondary' | 'text' | 'danger-text';
    height?: 28 | 32 | 34;
    icon?: IconName;
    type?: 'button' | 'submit' | 'reset';
    disabled?: boolean;
    title?: string;
    ariaLabel?: string;
    ariaDescribedby?: string;
    ariaExpanded?: boolean;
    ariaHaspopup?: 'menu' | 'dialog';
    onclick?: (event: MouseEvent) => void;
    children?: Snippet;
  }

  let {
    variant = 'secondary',
    height = 32,
    icon,
    type = 'button',
    disabled = false,
    title,
    ariaLabel,
    ariaDescribedby,
    ariaExpanded,
    ariaHaspopup,
    onclick,
    children,
  }: Props = $props();
</script>

<button
  class={variant}
  style:height="{height}px"
  {type}
  {disabled}
  {title}
  aria-label={ariaLabel}
  aria-describedby={ariaDescribedby}
  aria-expanded={ariaExpanded}
  aria-haspopup={ariaHaspopup}
  {onclick}
>
  {#if icon}
    <Icon name={icon} size={14} />
  {/if}
  {@render children?.()}
</button>

<style>
  button {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    padding: 0 14px;
    border: 1px solid transparent;
    border-radius: 4px;
    font-size: 13px;
    white-space: nowrap;
  }

  button:not(:disabled) {
    cursor: pointer;
  }

  .accent {
    background: var(--accent);
    border-color: var(--accent);
    color: var(--onAccent);
    font-weight: 600;
  }

  .secondary {
    background: var(--layer);
    border-color: var(--stroke);
    border-bottom-color: var(--strokeHi);
    color: var(--text);
  }

  .text,
  .danger-text {
    padding: 0 8px;
    background: transparent;
    border: none;
  }

  .text {
    color: var(--accent);
  }

  .danger-text {
    color: var(--err);
  }
</style>
