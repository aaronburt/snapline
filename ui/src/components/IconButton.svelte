<script lang="ts">
  import Icon from './Icon.svelte';
  import type { IconName } from '../lib/icons';

  interface Props {
    label: string;
    icon: IconName;
    size?: 32 | 34;
    pressed?: boolean;
    variant?: 'card' | 'plain';
    disabled?: boolean;
    title?: string;
    onclick?: (event: MouseEvent) => void;
  }

  let {
    label,
    icon,
    size = 34,
    pressed,
    variant = 'card',
    disabled = false,
    title,
    onclick,
  }: Props = $props();
</script>

<button
  type="button"
  class="hv {variant}"
  style:width="{size}px"
  style:height="{size}px"
  aria-label={label}
  aria-pressed={pressed}
  {disabled}
  {title}
  {onclick}
>
  <Icon name={icon} size={16} />
</button>

<style>
  button {
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 0;
    border-radius: 4px;
  }

  .card {
    background: var(--card);
    border: 1px solid var(--stroke);
    border-bottom-color: var(--strokeHi);
    color: var(--text);
  }

  .plain {
    background: transparent;
    border: none;
    color: var(--text);
  }
</style>
