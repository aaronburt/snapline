<script lang="ts">
  import Icon from './Icon.svelte';
  import type { IconName } from '../lib/icons';

  interface Props {
    value?: string;
    options: string[];
    icon: IconName;
    title: string;
    description?: string;
    onchange?: (value: string) => void;
  }

  let {
    value = $bindable(''),
    options,
    icon,
    title,
    description = '',
    onchange,
  }: Props = $props();

  const id = $props.id();
</script>

<div class="row">
  <Icon name={icon} size={20} />
  <div class="text">
    <span id={id} class="title">{title}</span>
    <span class="description">{description}</span>
  </div>
  <select
    aria-labelledby={id}
    bind:value
    onchange={(event) => onchange?.(event.currentTarget.value)}
  >
    {#each options as option}
      <option value={option}>{option}</option>
    {/each}
  </select>
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

  select {
    height: 34px;
    min-width: 220px;
    padding: 0 10px;
    border: 1px solid var(--stroke);
    border-bottom-color: var(--strokeHi);
    border-radius: 4px;
    background: var(--layer);
    color: var(--text);
    font-size: 13px;
  }
</style>
