<script lang="ts">
  import { onMount, tick } from 'svelte';
  import Badge from '../../../components/Badge.svelte';
  import Icon from '../../../components/Icon.svelte';
  import Thumb from '../../../components/Thumb.svelte';
  import { onEvent } from '../../../lib/events';
  import type { AppEvent } from '../../../lib/ipc/bindings/AppEvent';
  import type { HistoryCaptureDto } from '../../../lib/ipc/bindings/HistoryCaptureDto';
  import type { UploadReq } from '../../../lib/ipc/bindings/UploadReq';
  import type { UploadResp } from '../../../lib/ipc/bindings/UploadResp';
  import type { UploadState } from '../../../lib/ipc/bindings/UploadState';
  import { call } from '../../../lib/ipc/core';
  import { copyLink, listDestinations } from '../../../lib/ipc/upload';
  import type { DestinationView } from '../../../lib/ipc/upload';
  import {
    formatBytes,
    formatDay,
    formatDuration,
    formatTime,
    timed,
    typeLabel,
    typeTag,
  } from './data';

  interface Props {
    capture: HistoryCaptureDto | null;
    defaultDestination: string;
    oncopy: (id: number) => void;
    onshow: (id: number) => void;
    onopen: (id: number) => void;
    onannotate: (id: number) => void;
    onpin: (id: number) => void;
    ondelete: (id: number) => void;
    onupload: (id: number) => void;
  }

  type ShareStatus = 'uploaded' | 'local' | 'failed' | 'uploading';

  interface ShareView {
    status: ShareStatus;
    url: string | null;
    deletionUrl: string | null;
    error: string | null;
    destId: string | null;
    queueId: number | null;
    retryInMs: number | null;
    cancellable: boolean;
  }

  let { capture, defaultDestination, oncopy, onshow, onopen, onannotate, onpin, ondelete }: Props =
    $props();

  let copyFailed = $state(false);
  let actionError = $state('');
  let busy = $state(false);
  let live = $state<UploadState | null>(null);
  let queueIds = $state<Record<number, number>>({});
  let destinations = $state<DestinationView[]>([]);
  let destinationsLoaded = $state(false);
  let menuOpen = $state(false);
  let shownId = $state<number | null>(null);
  let menuTrigger = $state<HTMLButtonElement | null>(null);
  let menuElement = $state<HTMLDivElement | null>(null);

  const isTimed = $derived(capture !== null && timed(capture));
  const isImage = $derived(capture !== null && !timed(capture));

  function shareStatus(state: string | null | undefined): ShareStatus {
    if (state === 'done') {
      return 'uploaded';
    }
    if (state === 'failed') {
      return 'failed';
    }
    if (state === 'queued' || state === 'uploading' || state === 'backingOff') {
      return 'uploading';
    }
    return 'local';
  }

  function matchesLive(state: UploadState): boolean {
    if (!capture) {
      return false;
    }
    const known: number | undefined = queueIds[capture.id];
    if (known !== undefined && known === state.id) {
      return true;
    }
    if (state.captureId !== null && state.captureId === capture.id) {
      return true;
    }
    if (state.runId !== null && state.runId === String(capture.id)) {
      return true;
    }
    const url = capture.upload?.url;
    return url !== undefined && url !== null && state.link === url;
  }

  const share = $derived.by<ShareView>(() => {
    if (!capture) {
      return {
        status: 'local',
        url: null,
        deletionUrl: null,
        error: null,
        destId: null,
        queueId: null,
        retryInMs: null,
        cancellable: false,
      };
    }
    const upload = capture.upload;
    const known = queueIds[capture.id] ?? null;
    if (live !== null && matchesLive(live)) {
      return {
        status: shareStatus(live.status),
        url: live.link ?? upload?.url ?? null,
        deletionUrl: live.deletionLink ?? upload?.deletionUrl ?? null,
        error: live.error ?? upload?.error ?? null,
        destId: live.dest,
        queueId: live.id,
        retryInMs: live.retryInMs,
        cancellable: live.status === 'queued' || live.status === 'backingOff',
      };
    }
    if (upload) {
      return {
        status: shareStatus(upload.state),
        url: upload.url,
        deletionUrl: upload.deletionUrl,
        error: upload.error,
        destId: upload.destId,
        queueId: known,
        retryInMs: null,
        cancellable: upload.state === 'queued' && known !== null,
      };
    }
    return {
      status: 'local',
      url: null,
      deletionUrl: null,
      error: null,
      destId: null,
      queueId: null,
      retryInMs: null,
      cancellable: false,
    };
  });

  const dimensions = $derived(
    capture?.width !== null &&
      capture?.width !== undefined &&
      capture?.height !== null &&
      capture?.height !== undefined
      ? `${capture.width} × ${capture.height}`
      : '—',
  );
  const destination = $derived(share.destId === null ? '—' : hostLabel(share.destId));
  const canDeleteFromHost = $derived(
    share.status === 'uploaded' && (share.destId === 'imgur' || share.destId === 's3'),
  );
  const defaultUploadable = $derived(
    defaultDestination !== '' &&
      defaultDestination !== 'save_locally_only' &&
      defaultDestination !== 'none',
  );
  const uploadReason = $derived(
    !defaultUploadable
      ? 'No upload destination is configured.'
      : busy
        ? 'An upload is already being started.'
        : '',
  );

  $effect(() => {
    const id = capture?.id ?? null;
    if (id !== shownId) {
      shownId = id;
      live = null;
      menuOpen = false;
      actionError = '';
      copyFailed = false;
    }
  });

  onMount(() => {
    let unlisten: (() => void) | undefined;
    const handler = (event: AppEvent): void => {
      if (event.area !== 'upload' || event.event.kind !== 'stateChanged') {
        return;
      }
      const state = event.event.data;
      if (!capture || !matchesLive(state)) {
        return;
      }
      live = state;
      queueIds = { ...queueIds, [capture.id]: state.id };
      if (state.status === 'done' || state.status === 'failed' || state.status === 'cancelled') {
        actionError = '';
      }
    };
    void onEvent(handler)
      .then((stop) => {
        unlisten = stop;
      })
      .catch(() => undefined);
    return () => {
      unlisten?.();
    };
  });

  function hostLabel(id: string): string {
    if (id === 'imgur') {
      return 'Imgur';
    }
    if (id === 's3') {
      return 'Amazon S3';
    }
    return id;
  }

  function destinationLabel(id: string): string {
    if (id === 'save_locally_only') {
      return 'Save locally only';
    }
    if (id === 'none') {
      return 'None';
    }
    return hostLabel(id);
  }

  function describe(error: unknown): string {
    if (
      error !== null &&
      typeof error === 'object' &&
      'message' in error &&
      typeof (error as { message: unknown }).message === 'string'
    ) {
      return (error as { message: string }).message;
    }
    return 'The upload could not be started.';
  }

  async function enqueue(destination: string): Promise<void> {
    if (!capture || busy) {
      return;
    }
    const id = capture.id;
    busy = true;
    actionError = '';
    try {
      const response = await call<UploadReq, UploadResp>('ipc_upload', {
        kind: 'uploadElsewhere',
        data: { capture_id: id, dest: destination },
      });
      if (response.kind !== 'accepted') {
        throw new Error('The upload could not be started.');
      }
      queueIds = { ...queueIds, [id]: response.data.id };
      live = {
        id: response.data.id,
        captureId: id,
        dest: destination,
        runId: null,
        status: 'queued',
        attempts: 0,
        link: null,
        pageLink: null,
        deletionLink: null,
        error: null,
        retryInMs: null,
      } satisfies UploadState;
    } catch (error) {
      actionError = describe(error);
    } finally {
      busy = false;
    }
  }

  async function retry(): Promise<void> {
    if (!capture || busy) {
      return;
    }
    const queueId = share.queueId;
    const liveQueueId = live !== null && live.id === queueId;
    if (queueId !== null && liveQueueId) {
      busy = true;
      actionError = '';
      try {
        await call<UploadReq, UploadResp>('ipc_upload', {
          kind: 'retry',
          data: { id: queueId },
        });
        if (live !== null && live.id === queueId) {
          live = { ...live, status: 'queued', error: null, retryInMs: null };
        }
      } catch (error) {
        actionError = describe(error);
      } finally {
        busy = false;
      }
      return;
    }
    await enqueue(share.destId ?? defaultDestination);
  }

  async function cancel(): Promise<void> {
    if (!capture || busy) {
      return;
    }
    const queueId = share.queueId;
    if (queueId === null) {
      return;
    }
    busy = true;
    actionError = '';
    try {
      await call<UploadReq, UploadResp>('ipc_upload', {
        kind: 'cancel',
        data: { id: queueId },
      });
      if (live !== null && live.id === queueId) {
        live = { ...live, status: 'cancelled', retryInMs: null };
      }
    } catch (error) {
      actionError = describe(error);
    } finally {
      busy = false;
    }
  }

  async function toggleElsewhere(): Promise<void> {
    if (menuOpen) {
      closeMenu();
      return;
    }
    menuOpen = true;
    if (!destinationsLoaded) {
      try {
        destinations = (await listDestinations()).filter((item) => item.configured);
      } catch {
        destinations = [];
      }
      destinationsLoaded = true;
    }
    await tick();
    const first = menuItems()[0];
    if (first) {
      first.focus();
    } else {
      menuElement?.focus();
    }
  }

  function menuItems(): HTMLButtonElement[] {
    return Array.from(menuElement?.querySelectorAll<HTMLButtonElement>('[role="menuitem"]') ?? []);
  }

  function closeMenu(restore = true): void {
    if (!menuOpen) {
      return;
    }
    menuOpen = false;
    if (restore) {
      menuTrigger?.focus();
    }
  }

  function onMenuKey(event: KeyboardEvent): void {
    if (!menuOpen) {
      return;
    }
    if (event.key === 'Escape') {
      event.preventDefault();
      closeMenu();
      return;
    }
    if (event.key === 'Tab') {
      closeMenu(false);
      return;
    }
    const items = menuItems();
    if (items.length === 0) {
      return;
    }
    const current = items.indexOf(document.activeElement as HTMLButtonElement);
    let next = current;
    if (event.key === 'ArrowDown') {
      next = current < 0 ? 0 : (current + 1) % items.length;
    } else if (event.key === 'ArrowUp') {
      next = current < 0 ? items.length - 1 : (current + items.length - 1) % items.length;
    } else if (event.key === 'Home') {
      next = 0;
    } else if (event.key === 'End') {
      next = items.length - 1;
    } else {
      return;
    }
    event.preventDefault();
    items[next].focus();
  }

  function onOutside(event: PointerEvent): void {
    if (!menuOpen) {
      return;
    }
    const target = event.target as Node;
    if (menuElement?.contains(target) || menuTrigger?.contains(target)) {
      return;
    }
    menuOpen = false;
  }

  function chooseElsewhere(id: string): void {
    menuOpen = false;
    void enqueue(id);
    menuTrigger?.focus();
  }

  async function copyText(text: string | null | undefined): Promise<void> {
    if (!text) {
      return;
    }
    try {
      await copyLink(text);
      copyFailed = false;
    } catch {
      copyFailed = true;
    }
  }
</script>

<svelte:window onpointerdown={onOutside} onkeydown={onMenuKey} />

<aside aria-label="Capture details">
  {#if capture}
    <h2 class="heading">Details</h2>

    <Thumb
      id={capture.id}
      alt=""
      duration={capture.durationMs === null ? undefined : formatDuration(capture.durationMs)}
      tag={capture.durationMs === null ? undefined : typeTag(capture.kind, capture.name)}
    />

    <div class="identity">
      <div class="filename">{capture.name}</div>
      <div class="kind">
        {typeLabel(capture.kind, capture.name)} · {formatDay(capture.createdAt)} at {formatTime(
          capture.createdAt,
        )}
      </div>
    </div>

    <div class="actions">
      {#if isTimed}
        <button
          type="button"
          class="hv iconbtn"
          aria-label="Open"
          title="Open"
          onclick={() => onopen(capture.id)}
        >
          <Icon name="play" size={14} strokeWidth={1.8} />
        </button>
      {/if}
      {#if isImage}
        <button
          type="button"
          class="iconbtn accent"
          aria-label="Annotate"
          title="Annotate"
          onclick={() => onannotate(capture.id)}
        >
          <Icon name="edit" size={14} strokeWidth={1.8} />
        </button>
      {/if}
      <button
        type="button"
        class="hv iconbtn"
        aria-label="Copy image"
        title="Copy image"
        onclick={() => oncopy(capture.id)}
      >
        <Icon name="copy" size={14} strokeWidth={1.8} />
      </button>
      <button
        type="button"
        class="hv iconbtn"
        aria-label="Show in folder"
        onclick={() => onshow(capture.id)}
      >
        <Icon name="folder" size={14} strokeWidth={1.8} />
      </button>
      {#if isImage}
        <button
          type="button"
          class="hv iconbtn"
          aria-label="Pin to screen"
          onclick={() => onpin(capture.id)}
        >
          <Icon name="pin" size={14} strokeWidth={1.8} />
        </button>
      {/if}
      <button
        type="button"
        class="hv iconbtn danger"
        aria-label="Delete"
        onclick={() => ondelete(capture.id)}
      >
        <Icon name="trash" size={14} strokeWidth={1.8} />
      </button>
    </div>

    <section class="sharing" aria-label="Sharing">
      <div class="sharinghead">
        <h2 class="heading">Link</h2>
        <Badge status={share.status === 'uploaded' ? 'ok' : share.status} long />
      </div>

      {#if share.status === 'uploaded'}
        <div class="linkrow">
          <input readonly aria-label="Share link" value={share.url ?? ''} />
          <button
            type="button"
            class="hv linkbtn"
            aria-label="Copy link"
            onclick={() => void copyText(share.url)}
          >
            <Icon name="copy" size={14} strokeWidth={1.8} />
          </button>
        </div>
        {#if share.deletionUrl}
          <div class="deletion">
            <span>Deletion link saved</span>
            <button
              type="button"
              class="hv textbtn"
              onclick={() => void copyText(share.deletionUrl)}
            >
              Copy deletion link
            </button>
          </div>
        {:else if canDeleteFromHost}
          <div class="deletion">
            <span>Delete from {hostLabel(share.destId ?? '')}</span>
          </div>
        {/if}
      {:else if share.status === 'local'}
        <div class="note">Saved on this PC only. Upload it to get a shareable link.</div>
        <button
          type="button"
          class="upload"
          disabled={!defaultUploadable || busy}
          aria-describedby={uploadReason === '' ? undefined : 'upload-reason'}
          onclick={() => void enqueue(defaultDestination)}
        >
          Upload to {destinationLabel(defaultDestination)}
        </button>
        <span id="upload-reason" class="sr-only">{uploadReason}</span>
      {:else if share.status === 'failed'}
        <div class="note">
          Upload failed: {share.error ?? 'the destination did not accept the file.'}
        </div>
        <div class="retry">
          <button type="button" class="upload" disabled={busy} onclick={() => void retry()}>
            Retry
          </button>
          <button
            type="button"
            class="hv secondary"
            aria-haspopup="menu"
            aria-expanded={menuOpen}
            bind:this={menuTrigger}
            onclick={() => void toggleElsewhere()}
          >
            Upload elsewhere…
          </button>
          {#if menuOpen}
            {#if destinations.length === 0}
              <div class="menu menuempty" role="status" bind:this={menuElement}>
                No destinations configured.
              </div>
            {:else}
              <div
                class="menu"
                role="menu"
                aria-label="Upload elsewhere"
                tabindex="-1"
                bind:this={menuElement}
              >
                {#each destinations as item (item.id)}
                  <button
                    type="button"
                    class="menuitem"
                    role="menuitem"
                    onclick={() => chooseElsewhere(item.id)}
                  >
                    {item.name}
                  </button>
                {/each}
              </div>
            {/if}
          {/if}
        </div>
      {:else if share.status === 'uploading'}
        <div class="note">Uploading…</div>
        <progress class="bar" aria-label="Uploading"></progress>
        {#if share.cancellable}
          <button type="button" class="hv secondary" disabled={busy} onclick={() => void cancel()}>
            Cancel
          </button>
        {/if}
      {/if}
      {#if actionError}
        <div class="note error" role="status">{actionError}</div>
      {/if}
      {#if copyFailed}
        <div class="note" role="status">The link could not be copied.</div>
      {/if}
    </section>

    <section class="info" aria-label="File info">
      <h2 class="heading infoheading">Info</h2>
      <div class="inforow"><span>Dimensions</span><span>{dimensions}</span></div>
      {#if isTimed}
        <div class="inforow">
          <span>Duration</span><span>{formatDuration(capture.durationMs)}</span>
        </div>
      {/if}
      <div class="inforow"><span>File size</span><span>{formatBytes(capture.bytes)}</span></div>
      <div class="inforow">
        <span>Captured from</span><span>{capture.sourceApp ?? 'Desktop'}</span>
      </div>
      <div class="inforow"><span>Destination</span><span>{destination}</span></div>
      <div class="inforow"><span>Saved to</span><span>{capture.folder || '—'}</span></div>
    </section>
  {:else}
    <div class="empty">Select a capture to see its details.</div>
  {/if}
</aside>

<style>
  aside {
    flex: 1 0 320px;
    min-width: 280px;
    max-width: 360px;
    border-left: 1px solid var(--stroke);
    padding: 20px;
    display: flex;
    flex-direction: column;
    gap: 18px;
    background: var(--layer);
    align-self: stretch;
    overflow-y: auto;
  }

  .heading {
    margin: 0;
    font-size: 14px;
    font-weight: 600;
  }

  .identity {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
  }

  .filename {
    font-size: 15px;
    font-weight: 600;
    word-break: break-all;
  }

  .kind {
    font-size: 12px;
    color: var(--text3);
  }

  .actions {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
  }

  .iconbtn {
    width: 34px;
    height: 34px;
    padding: 0;
    border: 1px solid var(--stroke);
    border-bottom-color: var(--strokeHi);
    background: var(--card);
    color: var(--text);
    border-radius: 4px;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .iconbtn.accent {
    border-color: var(--accent);
    background: var(--accent);
    color: var(--onAccent);
  }

  .iconbtn.danger {
    color: var(--err);
  }

  .sharing {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding-top: 16px;
    border-top: 1px solid var(--stroke);
  }

  .sharinghead {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .linkrow {
    display: flex;
    gap: 6px;
  }

  .linkrow input {
    flex: 1;
    min-width: 0;
    height: 32px;
    padding: 0 10px;
    border: 1px solid var(--stroke);
    border-bottom-color: var(--strokeHi);
    border-radius: 4px;
    background: var(--layer);
    color: var(--text);
    font-size: 12px;
    font-family: var(--type-mono);
  }

  .linkbtn {
    width: 32px;
    height: 32px;
    border: 1px solid var(--stroke);
    background: var(--layer);
    color: var(--text);
    border-radius: 4px;
    display: flex;
    align-items: center;
    justify-content: center;
    flex: none;
  }

  .deletion {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    font-size: 12px;
    color: var(--text3);
  }

  .textbtn {
    height: 28px;
    padding: 0 8px;
    border: none;
    background: transparent;
    color: var(--accent);
    border-radius: 4px;
    font-size: 12px;
  }

  .note {
    font-size: 12px;
    color: var(--text2);
  }

  .upload {
    height: 32px;
    align-self: flex-start;
    padding: 0 12px;
    border: 1px solid var(--accent);
    background: var(--accent);
    color: var(--onAccent);
    border-radius: 4px;
    font-size: 12px;
    font-weight: 600;
    cursor: pointer;
  }

  .upload:disabled {
    opacity: 0.55;
    cursor: default;
  }

  .retry {
    position: relative;
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
  }

  .menu {
    position: absolute;
    top: calc(100% + 4px);
    left: 0;
    z-index: 5;
    min-width: 180px;
    max-width: 260px;
    padding: 4px;
    border: 1px solid var(--stroke);
    border-radius: 6px;
    background: var(--layer);
    box-shadow: 0 10px 24px var(--shadow);
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .menuitem {
    min-height: 30px;
    padding: 0 8px;
    border: none;
    border-radius: 4px;
    background: transparent;
    color: var(--text);
    font-size: 12px;
    text-align: left;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .menuitem:hover {
    background: var(--subtle);
  }

  .menuempty {
    padding: 6px 8px;
    font-size: 12px;
    color: var(--text3);
  }

  .bar {
    width: 100%;
    height: 4px;
    border: none;
    border-radius: 2px;
    overflow: hidden;
  }

  .error {
    color: var(--err);
  }

  .secondary {
    height: 32px;
    padding: 0 12px;
    border: 1px solid var(--stroke);
    background: var(--layer);
    color: var(--text);
    border-radius: 4px;
    font-size: 12px;
  }

  .info {
    display: flex;
    flex-direction: column;
  }

  .infoheading {
    margin: 0 0 6px;
    font-size: 13px;
  }

  .inforow {
    display: flex;
    justify-content: space-between;
    gap: 12px;
    padding: 7px 0;
    border-bottom: 1px solid var(--stroke);
    font-size: 12px;
  }

  .inforow span:first-child {
    color: var(--text3);
    flex: none;
  }

  .inforow span:last-child {
    color: var(--text);
    text-align: right;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .empty {
    margin: auto;
    font-size: 12px;
    color: var(--text3);
    text-align: center;
  }
</style>
