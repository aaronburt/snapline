<script lang="ts">
  import Icon from './Icon.svelte';
  import type { IconName } from '../lib/icons';

  interface Props {
    value?: number;
    min?: number;
    max?: number;
    step?: number;
    unit?: string;
    icon: IconName;
    title: string;
    description?: string;
    onchange?: (value: number) => void;
  }

  let {
    value = $bindable(0),
    min = 0,
    max = 100,
    step = 1,
    unit = '',
    icon,
    title,
    description = '',
    onchange,
  }: Props = $props();

  const id = $props.id();
  const shown = $derived(`${value}${unit}`);
</script>

<div class="row">
  <Icon name={icon} size={20} />
  <div class="text">
    <span {id} class="title">{title}</span>
    <span class="description">{description}</span>
  </div>
  <div class="range">
    <input
      type="range"
      {min}
      {max}
      {step}
      aria-labelledby={id}
      aria-valuetext={shown}
      bind:value
      oninput={(event) => onchange?.(event.currentTarget.valueAsNumber)}
    />
    <span class="shown">{shown}</span>
  </div>
</div>

<style>
  .row {
    min-height: 68px;
    padding: 12px 20px;
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 12px 16px;
    background: var(--card);
    border: 1px solid var(--stroke);
    border-radius: 6px;
  }

  .text {
    flex: 1 1 220px;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .title {
    font-size: 14px;
  }

  .description {
    font-size: 12px;
    color: var(--text3);
  }

  .range {
    display: flex;
    align-items: center;
    gap: 12px;
  }

  input {
    width: 200px;
    accent-color: var(--accent);
  }

  .shown {
    width: 44px;
    text-align: right;
    font-size: 13px;
    color: var(--text2);
  }
</style>
