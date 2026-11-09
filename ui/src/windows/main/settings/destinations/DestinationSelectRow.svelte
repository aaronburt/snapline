<script lang="ts">
  import Icon from '../../../../components/Icon.svelte';
  import type { IconName } from '../../../../lib/icons';

  interface Option {
    id: string;
    label: string;
  }

  interface Props {
    value: string;
    options: Option[];
    icon: IconName;
    title: string;
    onchange?: (value: string) => void;
  }

  let { value, options, icon, title, onchange }: Props = $props();
  const id = $props.id();
</script>

<div class="row">
  <Icon name={icon} size={20} />
  <div class="text">
    <span id={id} class="title">{title}</span>
  </div>
  <select
    aria-labelledby={id}
    {value}
    onchange={(event) => onchange?.(event.currentTarget.value)}
  >
    {#each options as option}
      <option value={option.id}>{option.label}</option>
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
