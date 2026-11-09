<script lang="ts">
  import { onDestroy } from 'svelte';
  import Logo from '../../components/Logo.svelte';
  import Icon from '../../components/Icon.svelte';
  import CaptionButtons from '../../components/CaptionButtons.svelte';
  import { currentRoute, navigate } from '../../router';
  import { searchQuery } from './history/search';

  interface Props {
    settings?: boolean;
  }

  let { settings = false }: Props = $props();

  let value = $state('');
  let timer: ReturnType<typeof setTimeout> | undefined;

  function apply(value: string): void {
    searchQuery.set(value);
    if ($currentRoute.page !== 'history') {
      navigate('/history');
    }
  }

  function oninput(event: Event): void {
    value = (event.currentTarget as HTMLInputElement).value;
    clearTimeout(timer);
    timer = setTimeout(() => {
      apply(value);
    }, 200);
  }

  onDestroy(() => {
    clearTimeout(timer);
  });
</script>

<div class="titlebar" data-tauri-drag-region>
  <Logo size={16} dot={false} />
  <span class="name">{settings ? 'Snapline Settings' : 'Snapline'}</span>
  {#if !settings}
    <div class="searchwrap">
      <label class="search">
        <Icon name="search" size={14} strokeWidth={2} />
        <span class="sr-only">Search captures</span>
        <input type="search" placeholder="Search by name, app, text in image…" {value} {oninput} />
      </label>
    </div>
  {:else}
    <div class="spacer"></div>
  {/if}
  <CaptionButtons />
</div>

<style>
  .titlebar {
    height: 44px;
    display: flex;
    align-items: center;
    gap: 12px;
    padding-left: 16px;
    flex: none;
  }

  .name {
    font-size: 12px;
    flex: none;
  }

  .searchwrap {
    flex: 1;
    min-width: 0;
    display: flex;
    justify-content: center;
  }

  .spacer {
    flex: 1;
  }

  .search {
    width: 100%;
    max-width: 420px;
    height: 32px;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 10px;
    background: var(--card);
    border: 1px solid var(--stroke);
    border-bottom-color: var(--strokeHi);
    border-radius: 4px;
    color: var(--text3);
  }

  input {
    flex: 1;
    min-width: 0;
    border: none;
    background: transparent;
    color: var(--text);
    font-size: 13px;
    outline: none;
  }
</style>
