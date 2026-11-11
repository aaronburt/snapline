<script lang="ts">
  import { currentRoute } from '../../../router';
  import GeneralPage from './GeneralPage.svelte';
  import CapturePage from './CapturePage.svelte';
  import HotkeysPage from './HotkeysPage.svelte';
  import UploadsPage from './UploadsPage.svelte';

  const page = $derived($currentRoute.page);
  let { query = '' }: { query?: string } = $props();

</script>

<div class="shell">
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
