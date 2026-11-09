<script lang="ts">
  import { onMount } from 'svelte';
  import type { RouteMatch } from '../../router';
  import { currentRoute } from '../../router';
  import { info } from '../../lib/ipc/app';
  import Nav from './Nav.svelte';
  import HistoryPage from './history/HistoryPage.svelte';
  import WorkflowsPage from './workflows/WorkflowsPage.svelte';
  import WorkflowEditor from './workflows/WorkflowEditor.svelte';
  import ToolsPage from './tools/ToolsPage.svelte';
  import QrTool from './tools/QrTool.svelte';
  import CombinerTool from './tools/CombinerTool.svelte';
  import ConverterTool from './tools/ConverterTool.svelte';
  import SettingsShell from './settings/SettingsShell.svelte';

  interface Props {
    route?: RouteMatch;
  }

  let { route }: Props = $props();

  const page = $derived($currentRoute.page);
  const isSettings = $derived(page.startsWith('settings-'));
  let hasMica = $state(false);

  onMount(() => {
    let disposed = false;
    void info()
      .then((response) => {
        if (!disposed && response.kind === 'info') {
          hasMica = response.data.hasMica;
        }
      })
      .catch(() => undefined);
    return () => {
      disposed = true;
    };
  });

  $effect(() => {
    if (route && route.page !== $currentRoute.page) {
      currentRoute.set(route);
    }
  });
</script>

<div class="root" class:mica={hasMica}>
  <div class="body">
    {#if isSettings}
      <SettingsShell />
    {:else}
      <Nav {page} />
      <div class="content">
        {#if page === 'history'}
          <HistoryPage />
        {:else if page === 'workflows'}
          <WorkflowsPage />
        {:else if page === 'workflow-editor'}
          <WorkflowEditor />
        {:else if page === 'tools'}
          <ToolsPage />
        {:else if page === 'qr'}
          <QrTool />
        {:else if page === 'combine'}
          <CombinerTool />
        {:else if page === 'convert'}
          <ConverterTool />
        {/if}
      </div>
    {/if}
  </div>
</div>

<style>
  .root {
    min-height: 100vh;
    height: 100vh;
    display: flex;
    flex-direction: column;
    background: var(--mica);
    color: var(--text);
    font-size: 13px;
    overflow: hidden;
  }

  .root.mica {
    background: transparent;
  }

  .body {
    flex: 1;
    display: flex;
    align-items: stretch;
    min-height: 0;
  }

  .content {
    flex: 999 1 560px;
    min-width: 0;
    min-height: 0;
    background: var(--layer);
    border: 1px solid var(--stroke);
    border-right: none;
    border-bottom: none;
    border-top-left-radius: 8px;
    overflow: auto;
    display: flex;
    align-items: stretch;
  }
</style>
