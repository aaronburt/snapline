<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import { onEvent } from '../../lib/events';
  import Icon from '../../components/Icon.svelte';
  import NavItem from '../../components/NavItem.svelte';
  import Toast from '../../components/Toast.svelte';
  import type { AppReq } from '../../lib/ipc/bindings/AppReq';
  import type { AppResp } from '../../lib/ipc/bindings/AppResp';
  import { call } from '../../lib/ipc/core';
  import { currentRoute, navigate } from '../../router';
  import { searchQuery } from './history/search';

  interface Props {
    page: string;
  }

  let { page }: Props = $props();

  let find = $state($searchQuery);
  let findTimer: ReturnType<typeof setTimeout> | undefined;
  let count = $state('');
  let captureError = $state('');

  const section = $derived(page.startsWith('workflow') ? 'workflows' : page);

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

  $effect(() => {
    if ($searchQuery !== find && findTimer === undefined) {
      find = $searchQuery;
    }
  });

  function onfind(event: Event): void {
    find = (event.currentTarget as HTMLInputElement).value;
    clearTimeout(findTimer);
    findTimer = setTimeout(() => {
      findTimer = undefined;
      searchQuery.set(find);
      if ($currentRoute.page !== 'history') {
        navigate('/history');
      }
    }, 200);
  }

  onDestroy(() => {
    clearTimeout(findTimer);
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
  <label class="find">
    <input
      type="search"
      aria-label="Search captures"
      placeholder="Search captures"
      value={find}
      oninput={onfind}
    />
    <Icon name="search" size={14} strokeWidth={2} />
  </label>
  <button type="button" class="newcapture" onclick={newCapture}>
    <Icon name="region" size={16} strokeWidth={1.8} />
    <span class="newlabel">New capture</span>
    <span class="chord">PrtSc</span>
  </button>

  <NavItem label="History" icon="history" {count} active={section === 'history'} href="#/history" />
  <NavItem label="Workflows" icon="workflows" active={section === 'workflows'} href="#/workflows" />

  <div class="footer">
    <h2 class="group">Settings</h2>
    <NavItem label="General" icon="gear" height={40} active={page === 'settings-general'} href="#/settings/general" />
    <NavItem label="Capture" icon="region" height={40} active={page === 'settings-capture'} href="#/settings/capture" />
    <NavItem label="Hotkeys" icon="grid" height={40} active={page === 'settings-hotkeys'} href="#/settings/hotkeys" />
    <NavItem label="Upload destinations" icon="cloud" height={40} active={page === 'settings-uploads'} href="#/settings/uploads" />
  </div>
</nav>

<Toast message={captureError} onclose={() => (captureError = '')} />

<style>
  nav {
    flex: 0 0 var(--sidebar-width);
    padding: 4px 12px 16px;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 2px;
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

  .group {
    margin: 8px 0 4px;
    padding: 0 12px;
    font-size: 12px;
    font-weight: 600;
    color: var(--text3);
  }

  .footer {
    margin-top: auto;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
</style>
