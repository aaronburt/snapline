<script lang="ts">
  import { onMount, tick } from 'svelte';
  import Badge from '../../../components/Badge.svelte';
  import Icon from '../../../components/Icon.svelte';
  import Thumb from '../../../components/Thumb.svelte';
  import UploadProgress from '../../../components/UploadProgress.svelte';
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
    pendingUploadLabel,
    timed,
    typeLabel,
    typeTag,
  } from './data';

  interface Props {
    width?: number;
    capture: HistoryCaptureDto | null;
    activeUpload: UploadState | null;
    defaultDestination: string;
    oncopy: (id: number) => void;
    onshow: (id: number) => void;
    onopen: (id: number) => void;
    onannotate: (id: number) => void;
    onpin: (id: number) => void;
    ondelete: (id: number) => void;
    onclose: () => void;
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
    progressAvailable: boolean;
    sent: number;
    total: number;
  }

  let {
    width = 320,
    capture,
    activeUpload,
    defaultDestination,
    oncopy,
    onshow,
    onopen,
    onannotate,
    onpin,
    ondelete,
    onclose,
  }: Props = $props();

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
  const isImage = $derived(capture?.kind === 'image');
  const currentLive = $derived(live !== null && matchesLive(live) ? live : activeUpload);

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
        progressAvailable: false,
        sent: 0,
        total: 0,
      };
    }
    const upload = capture.upload;
    const known = queueIds[capture.id] ?? null;
    if (currentLive !== null && matchesLive(currentLive)) {
      return {
        status: shareStatus(currentLive.status),
        url: currentLive.link ?? upload?.url ?? null,
        deletionUrl: currentLive.deletionLink ?? upload?.deletionUrl ?? null,
        error: currentLive.error ?? upload?.error ?? null,
        destId: currentLive.dest,
        queueId: currentLive.id,
        retryInMs: currentLive.retryInMs,
        cancellable: currentLive.cancellable,
        progressAvailable: currentLive.progressAvailable,
        sent: currentLive.sent,
        total: currentLive.total,
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
        progressAvailable: false,
        sent: upload.sent,
        total: upload.total,
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
      progressAvailable: false,
      sent: 0,
      total: 0,
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
    let disposed = false;
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
        if (disposed) stop();
        else unlisten = stop;
      })
      .catch(() => undefined);
    return () => {
      disposed = true;
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
    const fileName = capture.name;
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
      if (capture?.id !== id || (live?.captureId === id && live.id >= response.data.id)) {
        return;
      }
      queueIds = { ...queueIds, [id]: response.data.id };
      live = {
        id: response.data.id,
        captureId: id,
        fileName,
        dest: destination,
        runId: null,
        status: 'queued',
        cancellable: true,
        progressAvailable: false,
        attempts: 0,
        link: null,
        pageLink: null,
        deletionLink: null,
        error: null,
        retryInMs: null,
        sent: 0,
        total: 0,
        linkBatch: null,
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
      if (event.key === 'Escape' && !event.defaultPrevented) {
        event.preventDefault();
        onclose();
      }
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

<aside aria-label="Capture details" style:--detail-width="{width}px">
  {#if capture}
    <div class="detail-heading">
      <h2 class="heading">Details</h2>
      <button
        class="hv iconbtn"
        type="button"
        aria-label="Close details"
        title="Close details"
        onclick={onclose}
      >
        <Icon name="close" size={14} />
      </button>
    </div>

    <div class="preview">
      {#if capture.kind === 'image' || capture.kind === 'video' || capture.kind === 'gif'}
        <Thumb
          id={capture.id}
          alt=""
          duration={isTimed && capture.durationMs !== null
            ? formatDuration(capture.durationMs)
            : undefined}
          tag={isTimed ? typeTag(capture.kind, capture.name) : undefined}
        />
      {:else}
        <div class="file-preview" aria-hidden="true">{typeTag(capture.kind, capture.name)}</div>
      {/if}
    </div>

    <div class="identity">
      <div class="filename">{capture.name}</div>
      <div class="kind">
        {typeLabel(capture.kind, capture.name)} · {formatDay(capture.createdAt)} at {formatTime(
          capture.createdAt,
        )}
      </div>
    </div>

    <div class="actions">
      {#if !capture.missing && capture.owned}
        <button
          type="button"
          class="hv iconbtn"
          aria-label="Open file"
          title="Open file"
          onclick={() => onopen(capture.id)}
        >
          <Icon name={isTimed ? 'play' : 'open'} size={14} strokeWidth={1.8} />
        </button>
      {/if}
      {#if isImage}
        <button
          type="button"
          class="iconbtn accent"
          aria-label="Annotate"
          title="Annotate"
          disabled={capture.missing}
          onclick={() => onannotate(capture.id)}
        >
          <Icon name="edit" size={14} strokeWidth={1.8} />
        </button>
      {/if}
      {#if isImage}
        <button
          type="button"
          class="hv iconbtn"
          aria-label="Copy image"
          title="Copy image"
          disabled={capture.missing}
          onclick={() => oncopy(capture.id)}
        >
          <Icon name="copy" size={14} strokeWidth={1.8} />
        </button>
      {/if}
      <button
        type="button"
        class="hv iconbtn"
        aria-label="Show in folder"
        title="Show in folder"
        disabled={capture.missing}
        onclick={() => onshow(capture.id)}
      >
        <Icon name="folder" size={14} strokeWidth={1.8} />
      </button>
      {#if isImage}
        <button
          type="button"
          class="hv iconbtn"
          aria-label="Pin to screen"
          title="Pin to screen"
          disabled={capture.missing}
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

    {#if share.status !== 'local' || !capture.owned}
      <section class="sharing" aria-label="Sharing">
        <div class="sharinghead">
          <h2 class="heading">Sharing</h2>
          <Badge
            status={share.status === 'uploaded' ? 'ok' : share.status}
            label={pendingUploadLabel(currentLive?.status ?? capture.upload?.state)}
            long
          />
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
          <div class="retry">
            <button
              type="button"
              class="hv secondary"
              disabled={busy || capture.missing}
              aria-haspopup="menu"
              aria-expanded={menuOpen}
              bind:this={menuTrigger}
              onclick={() => void toggleElsewhere()}>Upload elsewhere…</button
            >
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
        {:else if share.status === 'failed'}
          <div class="note">
            Upload failed: {share.error ?? 'the destination did not accept the file.'}
          </div>
          <div class="retry">
            <button
              type="button"
              class="upload"
              disabled={busy || capture.missing}
              onclick={() => void retry()}
            >
              Retry
            </button>
            <button
              type="button"
              class="hv secondary"
              disabled={capture.missing}
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
          {#if (currentLive?.status ?? capture.upload?.state) === 'uploading'}
            <div class="note">
              {share.progressAvailable && share.total > 0
                ? `${formatBytes(share.sent)} of ${formatBytes(share.total)}`
                : 'Sending file…'}
            </div>
            <UploadProgress
              determinate={share.progressAvailable}
              sent={share.sent}
              total={share.total}
            />
          {/if}
          {#if share.cancellable}
            <button
              type="button"
              class="hv secondary"
              disabled={busy}
              onclick={() => void cancel()}
            >
              Cancel
            </button>
          {/if}
        {:else}
          {#if capture.missing}
            <div class="note">The original file is unavailable. Add it again before uploading.</div>
          {:else}
            <div class="note">This item is saved locally.</div>
            <button
              type="button"
              class="upload"
              disabled={busy}
              aria-haspopup="menu"
              aria-expanded={menuOpen}
              bind:this={menuTrigger}
              onclick={() => void toggleElsewhere()}>Upload…</button
            >
            {#if menuOpen}
              {#if destinations.length === 0}
                <div class="menu menuempty" role="status" bind:this={menuElement}>
                  No destinations configured.
                </div>
              {:else}
                <div
                  class="menu"
                  role="menu"
                  aria-label="Upload to a destination"
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
          {/if}
        {/if}
        {#if actionError}
          <div class="note error" role="status">{actionError}</div>
        {/if}
        {#if copyFailed}
          <div class="note" role="status">The link could not be copied.</div>
        {/if}
      </section>
    {/if}

    <section class="info" aria-label="File info">
      <h2 class="heading infoheading">Info</h2>
      <div class="inforow"><span>Dimensions</span><span>{dimensions}</span></div>
      {#if capture.missing}
        <div class="inforow"><span>Availability</span><span>Original file missing</span></div>
      {/if}
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
    flex: 0 0 var(--detail-width);
    min-width: 280px;
    max-width: 480px;
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

  .detail-heading {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .detail-heading .iconbtn {
    width: 30px;
    height: 30px;
  }

  @media (max-width: 1050px) {
    aside {
      flex: 0 0 34%;
      min-width: 0;
      max-width: none;
      border-left: 0;
      border-top: 1px solid var(--stroke);
      padding: 12px 24px 16px;
      display: grid;
      grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
      align-content: start;
      gap: 12px 20px;
    }

    .detail-heading,
    .info {
      grid-column: 1 / -1;
    }

    .preview {
      display: none;
    }

    .identity {
      grid-column: 1;
      grid-row: 2;
    }

    .actions {
      grid-column: 1;
      grid-row: 3;
    }

    aside .sharing {
      grid-column: 2;
      grid-row: 2 / span 2;
      padding-top: 0;
      border-top: 0;
    }
  }

  .identity {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
  }

  .file-preview {
    width: 100%;
    aspect-ratio: 16 / 10;
    display: flex;
    align-items: center;
    justify-content: center;
    border: 1px solid var(--stroke);
    border-radius: 4px;
    background: var(--subtle);
    color: var(--text2);
    font: 600 20px var(--type-mono);
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
