<script lang="ts">
  import { onMount } from 'svelte';
  import { onEvent } from '../../lib/events';
  import Icon from '../../components/Icon.svelte';
  import NavItem from '../../components/NavItem.svelte';
  import type { AppReq } from '../../lib/ipc/bindings/AppReq';
  import type { AppResp } from '../../lib/ipc/bindings/AppResp';
  import { call } from '../../lib/ipc/core';
  import { listDestinations, type DestinationView } from '../../lib/ipc/upload';
  import { loadSettings, settings, settingsError } from './settings/settings';

  interface Props {
    page: string;
  }

  let { page }: Props = $props();

  let count = $state('');
  let captureError = $state('');
  let destinations = $state<DestinationView[]>([]);
  let destinationState = $state<'loading' | 'ready' | 'error'>('loading');

  const section = $derived(page.startsWith('workflow') ? 'workflows' : ['qr', 'combine', 'convert'].includes(page) ? 'tools' : page);
  const destination = $derived(destinationState === 'loading' ? 'Checking destination…' : destinationState === 'error' ? 'Destination unavailable' : destinations.find((item) => item.id === $settings.uploads.images)?.name ?? $settings.uploads.images);

  async function loadCount(): Promise<void> {
    const req: AppReq = { kind: 'stats' };
    const response = await call<AppReq, AppResp>('ipc_app', req);
    if (response.kind === 'stats') {
      count = String(response.data.count);
    }
  }

  onMount(() => {
    const refreshCount = (): void => {
      void loadCount().catch(() => undefined);
    };
    const refresh = (): void => {
      refreshCount();
      void loadSettings().then(() => { destinationState = $settingsError === null ? 'ready' : 'error'; });
      void listDestinations().then((items) => { destinations = items; }).catch(() => undefined);
    };
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void onEvent((event) => {
      if (event.area === 'history' && event.event.kind !== 'ping') refreshCount();
    }).then((stop) => {
      if (disposed) stop();
      else unlisten = stop;
    }).catch(() => undefined);
    refresh();
    window.addEventListener('focus', refresh);
    return () => {
      disposed = true;
      unlisten?.();
      window.removeEventListener('focus', refresh);
    };
  });

  function newCapture(): void {
    const req: AppReq = { kind: 'panelAction', data: { action: 'capture_region' } };
    captureError = '';
    void call<AppReq, AppResp>('ipc_app', req).catch((error: unknown) => {
      captureError = typeof error === 'object' && error !== null && 'message' in error ? String(error.message) : 'Capture could not be completed.';
    });
  }
</script>

<nav aria-label="Main">
  <button type="button" class="newcapture" onclick={newCapture}>
    <Icon name="region" size={16} strokeWidth={1.8} />
    <span class="newlabel">New capture</span>
    <span class="chord">PrtSc</span>
  </button>
  {#if captureError}
    <p role="alert">{captureError}</p>
  {/if}

  <NavItem label="History" icon="history" {count} active={section === 'history'} href="#/history" />
  <NavItem label="Workflows" icon="workflows" active={section === 'workflows'} href="#/workflows" />
  <NavItem label="Tools" icon="tools" active={section === 'tools'} href="#/tools" />

  <div class="footer">
    <a
      class="hv destination"
      href="#/settings/uploads"
      aria-label={`Default destination: ${destination}. Change in settings`}
    >
      <Icon name="cloud" size={16} />
      {destination}
    </a>

    <NavItem label="Settings" icon="gear" href="#/settings/general" />
  </div>
</nav>

<style>
  nav {
    flex: 1 1 220px;
    max-width: 100%;
    padding: 4px 8px 16px;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .newcapture {
    height: 40px;
    margin: 0 4px 10px;
    border: 1px solid var(--accent);
    background: var(--accent);
    color: var(--onAccent);
    border-radius: 6px;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 0 12px;
    font-size: 13px;
    font-weight: 600;
    cursor: pointer;
  }

  .newlabel {
    flex: 1;
    text-align: left;
  }

  .chord {
    font-size: 11px;
    font-weight: 500;
    opacity: 0.85;
  }

  .footer {
    margin-top: auto;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .destination {
    margin: 0 4px 8px;
    min-height: 44px;
    padding: 0 12px;
    border-radius: 6px;
    background: var(--subtle);
    border: 1px solid var(--stroke);
    display: flex;
    align-items: center;
    gap: 10px;
    color: var(--text);
    font-size: 13px;
    font-weight: 500;
  }
</style>
