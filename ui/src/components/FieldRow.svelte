<script lang="ts">
  import Icon from './Icon.svelte';
  import type { IconName } from '../lib/icons';

  interface Props {
    value?: string;
    icon: IconName;
    title: string;
    description?: string;
    mono?: boolean;
    button?: string;
    onbutton?: () => void;
    placeholder?: string;
    inputType?: 'text' | 'search';
    onchange?: (value: string) => void;
  }

  let {
    value = $bindable(''),
    icon,
    title,
    description = '',
    mono = false,
    button,
    onbutton,
    placeholder,
    inputType = 'text',
    onchange,
  }: Props = $props();

  const id = $props.id();
</script>

<div class="row">
  <Icon name={icon} size={20} />
  <div class="text">
    <span {id} class="title">{title}</span>
    <span class="description">{description}</span>
  </div>
  <div class="field">
    <input
      type={inputType}
      aria-labelledby={id}
      class:mono
      bind:value
      {placeholder}
      spellcheck="false"
      oninput={(event) => onchange?.(event.currentTarget.value)}
    />
    {#if button}
      <button type="button" class="hv" onclick={onbutton}>{button}</button>
    {/if}
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

  .field {
    display: flex;
    gap: 6px;
    flex: 1 1 320px;
    max-width: 460px;
  }

  input {
    flex: 1;
    min-width: 0;
    height: 34px;
    padding: 0 10px;
    border: 1px solid var(--stroke);
    border-bottom-color: var(--strokeHi);
    border-radius: 4px;
    background: var(--layer);
    color: var(--text);
    font-size: 12px;
  }

  input.mono {
    font-family: var(--type-mono);
  }

  button {
    height: 34px;
    padding: 0 12px;
    border: 1px solid var(--stroke);
    border-bottom-color: var(--strokeHi);
    background: var(--layer);
    color: var(--text);
    border-radius: 4px;
    font-size: 13px;
    flex: none;
  }
</style>
