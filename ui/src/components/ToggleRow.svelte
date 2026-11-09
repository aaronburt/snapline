<script lang="ts">
  import Icon from './Icon.svelte';
  import type { IconName } from '../lib/icons';

  interface Props {
    checked?: boolean;
    icon: IconName;
    title: string;
    description?: string;
    onchange?: (checked: boolean) => void;
  }

  let {
    checked = $bindable(false),
    icon,
    title,
    description = '',
    onchange,
  }: Props = $props();

  const id = $props.id();
</script>

<label class="hv row" for={id}>
  <Icon name={icon} size={20} />
  <span class="text">
    <span class="title">{title}</span>
    <span class="description">{description}</span>
  </span>
  <span class="state">{checked ? 'On' : 'Off'}</span>
  <input
    id={id}
    type="checkbox"
    class="tg"
    bind:checked
    onchange={(event) => onchange?.(event.currentTarget.checked)}
  />
</label>

<style>
  .row {
    min-height: 68px;
    padding: 12px 20px;
    display: flex;
    align-items: center;
    gap: 16px;
    background: var(--card);
    border: 1px solid var(--stroke);
    border-radius: 6px;
  }

  .text {
    flex: 1;
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

  .state {
    font-size: 13px;
    color: var(--text2);
    width: 24px;
    text-align: right;
  }
</style>
