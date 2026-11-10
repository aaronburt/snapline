<script lang="ts">
  import { onMount, tick } from 'svelte';
  import Icon from '../../components/Icon.svelte';
  import Logo from '../../components/Logo.svelte';
  import Thumb from '../../components/Thumb.svelte';
  import { onEvent } from '../../lib/events';
  import type { UploadReq } from '../../lib/ipc/bindings/UploadReq';
  import type { UploadResp } from '../../lib/ipc/bindings/UploadResp';
  import type { UploadState } from '../../lib/ipc/bindings/UploadState';
  import { call } from '../../lib/ipc/core';
  import { cancelUpload, listUploadStates } from '../../lib/ipc/upload';
  import { panelCaptures, toPanelRecent } from './data';
  import type { PanelRecent } from './data';
  import {
    openCapture,
    openMain,
    panelAction,
    panelClose,
    panelCopyLink,
    panelRecent,
    panelResize,
  } from './ipc';

  interface Props {
    lastLink?: string | null;
  }

  let { lastLink }: Props = $props();

  let frame: HTMLElement | undefined = $state();
  let recents = $state<PanelRecent[]>([]);
  let activeUploads = $state<UploadState[]>([]);
  let recentsLoaded = $state(false);
  let lastLinkFromIpc = $state<string | null>(null);

  const source = $derived(lastLink !== undefined ? lastLink : lastLinkFromIpc);
  const link = $derived(source === null || source === '' ? null : withoutScheme(source));

  function withoutScheme(value: string): string {
    if (value.startsWith('https://')) {
      return value.slice(8);
    }
    if (value.startsWith('http://')) {
      return value.slice(7);
    }
    return value;
  }

  async function reportHeight(): Promise<void> {
    await tick();
    if (!frame) {
      return;
    }
    const height = Math.ceil(frame.getBoundingClientRect().height);
    await panelResize(height).catch(() => undefined);
  }

  async function loadRecent(): Promise<void> {
    try {
      const captures = await panelRecent();
      recents = captures.map((capture) => toPanelRecent(capture));
    } catch {
      recents = [];
    } finally {
      recentsLoaded = true;
      await reportHeight();
    }
  }

  async function loadLastLink(): Promise<void> {
    try {
      const response = await call<UploadReq, UploadResp>('ipc_upload', {
        kind: 'lastLink',
      });
      if (response.kind === 'lastLink') {
        lastLinkFromIpc = response.data.link;
      }
    } catch {
      lastLinkFromIpc = null;
    }
    await reportHeight();
  }

  onMount(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    let observed: Set<number> | null = new Set();
    void onEvent((event) => {
      if (event.area !== 'upload' || event.event.kind !== 'stateChanged') {
        return;
      }
      const state = event.event.data;
      observed?.add(state.id);
      const before = activeUploads.length;
      if (isActiveUpload(state)) {
        if (activeUploads.some((upload) => upload.id === state.id)) {
          activeUploads = activeUploads.map((upload) => upload.id === state.id ? state : upload);
        } else {
          activeUploads = [...activeUploads, state];
        }
      } else {
        activeUploads = activeUploads.filter((upload) => upload.id !== state.id);
      }
      if (activeUploads.length !== before) {
        void reportHeight();
      }
    }).then((stop) => {
      if (disposed) stop();
      else unlisten = stop;
    }).catch(() => undefined).then(async () => {
      if (disposed) return;
      const states = await listUploadStates();
      if (disposed) return;
      activeUploads = [
        ...states.filter((state) => !observed?.has(state.id) && isActiveUpload(state)),
        ...activeUploads,
      ];
      await reportHeight();
    }).catch(() => undefined).finally(() => { observed = null; });
    void reportHeight();
    void loadRecent();
    void loadLastLink();
    return () => {
      disposed = true;
      unlisten?.();
    };
  });

  function isActiveUpload(state: UploadState): boolean {
    return state.status === 'queued' || state.status === 'uploading' || state.status === 'backingOff';
  }

  function uploadName(state: UploadState): string {
    return state.fileName || recents.find((capture) => capture.id === state.captureId)?.name || 'File upload';
  }

  function formatSize(bytes: number): string {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  }

  function close(): void {
    void panelClose().catch(() => undefined);
  }

  function onKeydown(event: KeyboardEvent): void {
    if (event.key === 'Escape') {
      event.preventDefault();
      close();
    }
  }

  function runAction(action: string): void {
    void panelAction(action).catch(() => close());
  }

  function showRoute(route: string): void {
    void openMain(route).catch(() => close());
  }

  function showCapture(id: number): void {
    void openCapture(id).catch(() => close());
  }

  function copyLastLink(): void {
    if (!link) {
      return;
    }
    void panelCopyLink().catch(() => undefined);
  }

  function cancel(id: number): void {
    void cancelUpload(id).catch(() => undefined);
  }
</script>

<svelte:window onkeydown={onKeydown} />

<main class="panel" aria-label="Snapline quick panel" bind:this={frame}>
  <header class="header">
    <Logo size={32} />
    <div class="title">Snapline</div>
    <button
      type="button"
      class="icon-button hv"
      aria-label="Open Snapline"
      onclick={() => showRoute('/history')}
    >
      <Icon name="open" size={18} />
    </button>
    <button
      type="button"
      class="icon-button hv"
      aria-label="Settings"
      onclick={() => showRoute('/settings/general')}
    >
      <Icon name="gearDetail" size={18} />
    </button>
  </header>

  <div class="tiles">
    {#each panelCaptures as capture (capture.id)}
      <button
        type="button"
        class="tile hv"
        class:accent={capture.accent === true}
        class:card={capture.accent !== true}
        title={`${capture.label} (${capture.chord})`}
        onclick={() => runAction(capture.action)}
      >
        <Icon name={capture.icon} size={20} />
        <span>{capture.label}</span>
      </button>
    {/each}
  </div>

  <section class="recent" aria-labelledby="recent-title">
    <div class="recent-header">
      <div class="recent-title" id="recent-title">Recent</div>
      <button type="button" class="view-all hv" onclick={() => showRoute('/history')}>
        View all
      </button>
    </div>
    {#if recentsLoaded && recents.length === 0}
      <div class="empty">No captures yet</div>
    {:else}
      <div class="recent-grid">
        {#each recents as recent (recent.id)}
          <button
            type="button"
            class="recent-item hv"
            aria-label={recent.name}
            onclick={() => showCapture(recent.id)}
          >
            <Thumb id={recent.id} alt={recent.name} duration={recent.duration} small />
            <span class="recent-meta">
              {#if recent.uploaded}
                <span class="uploaded" role="img" aria-label="Uploaded">
                  <Icon name="link" size={12} />
                </span>
              {/if}
              {recent.time}
            </span>
          </button>
        {/each}
      </div>
    {/if}
  </section>

  {#if activeUploads.length > 0}
    <section class="uploads" aria-labelledby="uploads-title">
      <div class="uploads-title" id="uploads-title">Uploads</div>
      {#each activeUploads as upload (upload.id)}
        <div class="upload-row">
          <div class="upload-details">
            <span class="upload-name">{uploadName(upload)}</span>
            <span class="upload-size">
              {upload.progressAvailable
                ? `${formatSize(upload.sent)} of ${formatSize(upload.total)}`
                : 'Uploading'}
            </span>
            <progress
              aria-label={`Upload progress for ${uploadName(upload)}`}
              value={upload.progressAvailable && upload.total > 0 ? upload.sent : undefined}
              max={upload.progressAvailable && upload.total > 0 ? upload.total : undefined}
            ></progress>
          </div>
          {#if upload.cancellable}
            <button type="button" class="cancel-upload hv" onclick={() => cancel(upload.id)}>Cancel</button>
          {/if}
        </div>
      {/each}
    </section>
  {/if}

  <footer class="footer">
    <span class="last-link-label">Last link</span>
    <span class="last-link" aria-live="polite">{link ?? 'None yet'}</span>
    <span id="last-link-reason" class="sr-only">
      {link ? 'Copies the last uploaded link to the clipboard.' : 'There is no link to copy yet.'}
    </span>
    <button
      type="button"
      class="copy hv"
      disabled={!link}
      aria-describedby="last-link-reason"
      onclick={copyLastLink}
    >
      <Icon name="copy" size={14} />
      Copy
    </button>
  </footer>
</main>

<style>
  .panel {
    width: 344px;
    background: var(--layer);
    border: 1px solid var(--stroke);
    border-radius: 8px;
    box-shadow: 0 16px 40px var(--shadow);
    overflow: hidden;
    display: flex;
    flex-direction: column;
  }

  .header {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 14px 12px 12px 16px;
  }

  .title {
    flex: 1;
    min-width: 0;
    font-size: 14px;
    font-weight: 600;
  }

  .icon-button {
    width: 36px;
    height: 36px;
    border: none;
    border-radius: 6px;
    padding: 0;
    background: transparent;
    color: var(--text2);
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .tiles {
    padding: 0 12px 12px;
    display: grid;
    grid-template-columns: repeat(4, minmax(0, 1fr));
    gap: 6px;
  }

  .tile {
    height: 56px;
    border-radius: 6px;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 4px;
    padding: 0 4px;
    font-size: 12px;
  }

  .tile.accent {
    background: var(--accent);
    color: var(--onAccent);
    border: 1px solid var(--accent);
  }

  .tile.card {
    background: var(--card);
    color: var(--text);
    border: 1px solid var(--stroke);
  }

  .tile span {
    font-weight: 500;
    white-space: nowrap;
  }

  .recent {
    padding: 10px 16px 12px;
    border-top: 1px solid var(--stroke);
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .recent-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .recent-title {
    font-size: 12px;
    color: var(--text3);
    font-weight: 500;
  }

  .view-all {
    font-size: 12px;
    color: var(--accent);
    padding: 4px 0;
    border: none;
    background: transparent;
  }

  .recent-grid {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 8px;
  }

  .recent-item {
    background: transparent;
    border: none;
    padding: 0;
    color: inherit;
    text-align: left;
    display: flex;
    flex-direction: column;
    gap: 5px;
    border-radius: 4px;
  }

  .recent-meta {
    display: flex;
    align-items: center;
    gap: 4px;
    font-size: 11px;
    color: var(--text3);
  }

  .uploaded {
    display: inline-flex;
    color: var(--ok);
  }

  .uploads {
    padding: 10px 16px 12px;
    border-top: 1px solid var(--stroke);
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .uploads-title {
    font-size: 12px;
    color: var(--text3);
    font-weight: 500;
  }

  .upload-row {
    display: flex;
    align-items: center;
    gap: 10px;
    min-width: 0;
  }

  .upload-details {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 3px;
  }

  .upload-name,
  .upload-size {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .upload-name {
    font-size: 12px;
    color: var(--text);
  }

  .upload-size {
    font-size: 10px;
    color: var(--text3);
  }

  .upload-details progress {
    width: 100%;
    height: 4px;
    accent-color: var(--accent);
  }

  .cancel-upload {
    flex: none;
    border: 1px solid var(--stroke);
    border-radius: 4px;
    background: var(--card);
    color: var(--text2);
    padding: 4px 7px;
    font-size: 11px;
  }

  .empty {
    font-size: 12px;
    color: var(--text3);
  }

  .footer {
    padding: 8px 8px 8px 16px;
    background: var(--mica);
    border-top: 1px solid var(--stroke);
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .last-link-label {
    font-size: 12px;
    color: var(--text3);
    flex: none;
  }

  .last-link {
    flex: 1;
    min-width: 0;
    font-size: 12px;
    color: var(--text2);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    font-family: var(--type-mono);
  }

  .copy {
    height: 32px;
    padding: 0 10px;
    border: 1px solid var(--stroke);
    background: var(--card);
    color: var(--text);
    border-radius: 4px;
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
  }

  .copy:disabled {
    cursor: default;
  }

  .copy:disabled:hover {
    box-shadow: none;
  }
</style>
