<script lang="ts">
  import { onMount, tick } from 'svelte';
  import './styles/tokens.css';
  import './styles/base.css';
  import { currentRoute, startRouter } from './router';
  import * as app from './lib/ipc/app';
  import QuickPanel from './windows/panel/QuickPanel.svelte';
  import MainShell from './windows/main/MainShell.svelte';

  onMount(() => {
    startRouter();
    void tick()
      .then(() => app.ready())
      .catch(() => undefined);
  });
</script>

{#if $currentRoute.page === 'panel'}
  <QuickPanel />
{:else}
  <MainShell route={$currentRoute} />
{/if}
