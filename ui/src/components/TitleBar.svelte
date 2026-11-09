<script lang="ts">
  import Logo from './Logo.svelte';
  import Icon from './Icon.svelte';
  import CaptionButtons from './CaptionButtons.svelte';

  interface Props {
    value?: string;
    oninput?: (event: Event) => void;
  }

  let { value = $bindable(''), oninput }: Props = $props();
</script>

<div class="titlebar" data-tauri-drag-region>
  <Logo size={16} dot={false} />
  <span class="name">Snapline</span>
  <div class="searchwrap">
    <label class="search">
      <Icon name="search" size={14} strokeWidth={2} />
      <span class="sr-only">Search captures</span>
      <input
        type="search"
        placeholder="Search by name, app, text in image…"
        bind:value
        {oninput}
      />
    </label>
  </div>
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
