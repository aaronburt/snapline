<script lang="ts">
  import { currentRoute } from '../../../router';
  import Icon from '../../../components/Icon.svelte';
  import NavItem from '../../../components/NavItem.svelte';
  import GeneralPage from './GeneralPage.svelte';
  import CapturePage from './CapturePage.svelte';
  import HotkeysPage from './HotkeysPage.svelte';
  import UploadsPage from './UploadsPage.svelte';
  import { lastMainRoute } from './lastMainRoute';
  import { searchSettings } from './registry';

  const page = $derived($currentRoute.page);
  let query = $state('');

  const otherMatches = $derived(
    searchSettings(query).filter((entry) => entry.page !== page),
  );
</script>

<div class="shell">
  <nav aria-label="Settings">
    <a class="hv back" href={`#${$lastMainRoute}`}>
      <Icon name="back" size={16} strokeWidth={1.8} />
      Back to Snapline
    </a>
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
    <NavItem label="General" icon="gear" height={40} active={page === 'settings-general'} href="#/settings/general" />
    <NavItem label="Capture" icon="region" height={40} active={page === 'settings-capture'} href="#/settings/capture" />
    <NavItem label="Hotkeys" icon="grid" height={40} active={page === 'settings-hotkeys'} href="#/settings/hotkeys" />
    <NavItem
      label="Upload destinations"
      icon="cloud"
      height={40}
      active={page === 'settings-uploads'}
      href="#/settings/uploads"
    />
  </nav>

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
    flex-wrap: wrap;
    align-items: flex-start;
    min-height: 0;
    height: 100%;
  }

  nav {
    display: flex;
    flex: 1 1 240px;
    flex-direction: column;
    gap: 4px;
    max-width: 100%;
    padding: 8px 12px 16px;
    box-sizing: border-box;
  }

  .back {
    display: flex;
    align-items: center;
    gap: 10px;
    align-self: flex-start;
    height: 36px;
    padding: 0 10px;
    border-radius: 4px;
    color: var(--text2);
  }

  .find {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 34px;
    margin: 8px 0 12px;
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
    flex: 999 1 560px;
    min-width: 0;
    height: 100%;
    overflow-y: auto;
    background: var(--layer);
    border-top: 1px solid var(--stroke);
    border-left: 1px solid var(--stroke);
    border-top-left-radius: 8px;
    padding: 28px 40px 48px;
    box-sizing: border-box;
  }

  .inner {
    display: flex;
    flex-direction: column;
    gap: 28px;
    max-width: 1000px;
  }
</style>
