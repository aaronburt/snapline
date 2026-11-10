<script lang="ts">
  import { currentRoute } from '../../../router';
  import Icon from '../../../components/Icon.svelte';
  import GeneralPage from './GeneralPage.svelte';
  import CapturePage from './CapturePage.svelte';
  import HotkeysPage from './HotkeysPage.svelte';
  import UploadsPage from './UploadsPage.svelte';
  import { searchSettings } from './registry';

  const page = $derived($currentRoute.page);
  let query = $state('');

  const otherMatches = $derived(searchSettings(query).filter((entry) => entry.page !== page));
</script>

<div class="shell">
  <div class="topbar">
    <label class="find">
      <input
        type="search"
        aria-label="Find a setting"
        placeholder="Find a setting"
        bind:value={query}
      />
      <Icon name="search" size={14} strokeWidth={2} />
    </label>
    {#if query.trim() !== '' && otherMatches.length > 0}
      <div class="other">
        <h2>On other pages</h2>
        {#each otherMatches as entry}
          <a class="hv" href={`#${entry.route}`}>
            {entry.title}
            <span>{entry.pageLabel}</span>
          </a>
        {/each}
      </div>
    {/if}
  </div>

  <main class="content">
    <div class="inner">
      {#if page === 'settings-general'}
        <GeneralPage filter={query} />
      {:else if page === 'settings-capture'}
        <CapturePage filter={query} />
      {:else if page === 'settings-hotkeys'}
        <HotkeysPage filter={query} />
      {:else if page === 'settings-uploads'}
        <UploadsPage filter={query} />
      {/if}
    </div>
  </main>
</div>

<style>
  .shell {
    display: flex;
    flex: 1;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
  }

  .topbar {
    flex: none;
    padding: 8px 40px 0;
  }

  .find {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 34px;
    max-width: 420px;
    margin: 0 0 8px;
    padding: 0 10px;
    background: var(--card);
    border: 1px solid var(--stroke);
    border-bottom-color: var(--strokeHi);
    border-radius: 4px;
    color: var(--text3);
  }

  .find input {
    flex: 1;
    min-width: 0;
    border: none;
    background: transparent;
    color: var(--text);
    font-size: 13px;
    outline: none;
  }

  .other {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 0 0 8px;
  }

  .other h2 {
    margin: 0 0 4px;
    padding: 0 10px;
    font-size: 12px;
    font-weight: 600;
    color: var(--text3);
  }

  .other a {
    display: flex;
    flex-direction: column;
    gap: 1px;
    padding: 6px 10px;
    border-radius: 4px;
    font-size: 12px;
    color: var(--text);
  }

  .other a span {
    font-size: 11px;
    color: var(--text3);
  }

  .content {
    flex: 1;
    min-width: 0;
    overflow-y: auto;
    padding: 20px 40px 48px;
  }

  .inner {
    display: flex;
    flex-direction: column;
    gap: 28px;
    max-width: 1000px;
  }
</style>
