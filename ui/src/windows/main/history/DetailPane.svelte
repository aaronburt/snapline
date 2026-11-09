<script lang="ts">
  import Badge from '../../../components/Badge.svelte';
  import Icon from '../../../components/Icon.svelte';
  import Thumb from '../../../components/Thumb.svelte';
  import type { HistoryCaptureDto } from '../../../lib/ipc/bindings/HistoryCaptureDto';
  import {
    badgeStatus,
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

  let {
    capture,
    defaultDestination,
    oncopy,
    onshow,
    onopen,
    onannotate,
    onpin,
    ondelete,
    onupload,
  }: Props = $props();

  let copyFailed = $state(false);

  const status = $derived(capture ? badgeStatus(capture) : 'local');
  const isTimed = $derived(capture !== null && timed(capture));
  const isImage = $derived(capture !== null && !timed(capture));
  const uploaded = $derived(status === 'ok');
  const local = $derived(status === 'local');
  const failed = $derived(status === 'failed');
  const uploading = $derived(status === 'uploading');
  const dimensions = $derived(
    capture?.width !== null &&
      capture?.width !== undefined &&
      capture?.height !== null &&
      capture?.height !== undefined
      ? `${capture.width} × ${capture.height}`
      : '—',
  );
  const destination = $derived(
    capture?.upload ? hostLabel(capture.upload.destId) : '—',
  );
  const deletion = $derived.by(() => {
    const upload = capture?.upload;
    if (!upload || !uploaded) {
      return false;
    }
    return upload.deletionUrl !== null || upload.destId === 'imgur' || upload.destId === 's3';
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

  async function copyText(text: string | null | undefined): Promise<void> {
    if (!text || !navigator.clipboard) {
      return;
    }
    try {
      await navigator.clipboard.writeText(text);
      copyFailed = false;
    } catch {
      copyFailed = true;
    }
  }
</script>

<aside aria-label="Capture details">
  {#if capture}
    <h2 class="heading">Details</h2>

    <Thumb
      id={capture.id}
      alt={capture.name}
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
        <Badge status={status} long />
      </div>

      {#if uploaded}
        <div class="linkrow">
          <input readonly aria-label="Share link" value={capture.upload?.url ?? ''} />
          <button
            type="button"
            class="hv linkbtn"
            aria-label="Copy link"
            onclick={() => void copyText(capture.upload?.url)}
          >
            <Icon name="copy" size={14} strokeWidth={1.8} />
          </button>
          <button type="button" class="hv linkbtn" aria-label="Show QR code">
            <Icon name="qr" size={14} strokeWidth={1.8} />
          </button>
        </div>
        {#if deletion}
          <div class="deletion">
            {#if capture.upload?.deletionUrl}
              <span>Deletion link saved</span>
              <button
                type="button"
                class="hv textbtn"
                onclick={() => void copyText(capture.upload?.deletionUrl)}
              >
                Copy deletion link
              </button>
            {:else}
              <span>Delete from {destination}</span>
            {/if}
          </div>
        {/if}
      {:else if local}
        <div class="note">Saved on this PC only. Upload it to get a shareable link.</div>
        <button type="button" class="upload" onclick={() => onupload(capture.id)}>
          Upload to {defaultDestination}
        </button>
      {:else if failed}
        <div class="note">
          Upload failed: {capture.upload?.error ?? 'the destination did not accept the file.'}
        </div>
        <div class="retry">
          <button type="button" class="upload" onclick={() => onupload(capture.id)}>Retry</button>
          <button type="button" class="hv secondary" onclick={() => onupload(capture.id)}>
            Upload elsewhere…
          </button>
        </div>
      {:else if uploading}
        <div class="note">Uploading…</div>
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

  .retry {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
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
