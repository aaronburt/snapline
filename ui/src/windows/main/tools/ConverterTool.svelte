<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import Button from '../../../components/Button.svelte';
  import Icon from '../../../components/Icon.svelte';
  import Segmented from '../../../components/Segmented.svelte';
  import SelectRow from '../../../components/SelectRow.svelte';
  import Thumb from '../../../components/Thumb.svelte';
  import type { ConvertFormatDto } from '../../../lib/ipc/bindings/ConvertFormatDto';
  import type { ConvertProgressDto } from '../../../lib/ipc/bindings/ConvertProgressDto';
  import type { ConvertSource } from '../../../lib/ipc/bindings/ConvertSource';
  import type { ConvertedDto } from '../../../lib/ipc/bindings/ConvertedDto';
  import type { HistoryCaptureDto } from '../../../lib/ipc/bindings/HistoryCaptureDto';
  import { historyPage } from '../../../lib/ipc/history';
  import {
    convertCancel,
    convertPickFile,
    convertProgress,
    convertResult,
    convertStart,
  } from '../../../lib/ipc/tool';
  import { formatDuration } from '../history/data';

  const formatOptions = ['GIF', 'WebM (VP9)', 'MP4 (smaller)'];
  const pollDelay = 250;

  let mode = $state('History');
  let captures = $state<HistoryCaptureDto[]>([]);
  let selectedId = $state<number | null>(null);
  let filePath = $state('');
  let formatLabel = $state('GIF');
  let jobId = $state<number | null>(null);
  let progress = $state<ConvertProgressDto | null>(null);
  let running = $state(false);
  let cancelling = $state(false);
  let result = $state<ConvertedDto | null>(null);
  let errorMessage = $state('');
  let historyMessage = $state('');
  let loadingHistory = $state(false);
  let timer: ReturnType<typeof setTimeout> | undefined;

  const ready = $derived(mode === 'History' ? selectedId !== null : filePath.trim().length > 0);
  const percent = $derived(progress?.percent ?? null);
  const rounded = $derived(percent === null ? null : Math.round(percent));
  const phaseLabel = $derived.by(() => {
    const current = progress;
    if (!current) {
      return 'Preparing';
    }
    if (current.phase === 'saving') {
      return 'Saving to History';
    }
    if (current.phase === 'done') {
      return 'Converted';
    }
    if (current.phase === 'failed') {
      return 'Failed';
    }
    if (current.phase === 'cancelled') {
      return 'Cancelled';
    }
    return 'Converting';
  });

  function messageOf(error: unknown): string {
    if (typeof error === 'object' && error !== null && 'message' in error) {
      const message = (error as { message?: unknown }).message;
      if (typeof message === 'string' && message.length > 0) {
        return message;
      }
    }
    return 'The request could not be completed.';
  }

  function outputFormat(): ConvertFormatDto {
    if (formatLabel === 'WebM (VP9)') {
      return 'webmVp9';
    }
    if (formatLabel === 'MP4 (smaller)') {
      return 'mp4Small';
    }
    return 'gif';
  }

  function source(): ConvertSource | null {
    if (mode === 'History') {
      return selectedId === null ? null : { kind: 'capture', data: { captureId: selectedId } };
    }
    const path = filePath.trim();
    return path.length === 0 ? null : { kind: 'file', data: { path } };
  }

  async function loadHistory(): Promise<void> {
    loadingHistory = true;
    historyMessage = '';
    try {
      const page = await historyPage('videos', null);
      captures = page.items;
      selectedId = page.items[0]?.id ?? null;
    } catch (error) {
      captures = [];
      selectedId = null;
      historyMessage = messageOf(error);
    } finally {
      loadingHistory = false;
    }
  }

  async function poll(id: number): Promise<void> {
    if (timer !== undefined) {
      clearTimeout(timer);
      timer = undefined;
    }
    if (jobId !== id) {
      return;
    }
    try {
      const next = await convertProgress(id);
      if (jobId !== id) {
        return;
      }
      progress = next;
      if (next.phase === 'failed') {
        errorMessage = next.message ?? 'The video could not be converted.';
        running = false;
        cancelling = false;
        return;
      }
      if (next.phase === 'cancelled') {
        errorMessage = '';
        running = false;
        cancelling = false;
        return;
      }
      if (next.phase === 'done') {
        try {
          result = await convertResult(id);
        } catch (error) {
          errorMessage = messageOf(error);
        }
        running = false;
        cancelling = false;
        return;
      }
      timer = setTimeout(() => {
        void poll(id);
      }, pollDelay);
    } catch (error) {
      if (jobId !== id) {
        return;
      }
      errorMessage = messageOf(error);
      running = false;
      cancelling = false;
    }
  }

  async function start(): Promise<void> {
    const chosen = source();
    if (!chosen) {
      errorMessage = 'Choose a video to convert.';
      return;
    }
    errorMessage = '';
    result = null;
    progress = null;
    cancelling = false;
    running = true;
    try {
      const id = await convertStart(chosen, outputFormat());
      jobId = id;
      await poll(id);
    } catch (error) {
      running = false;
      errorMessage = messageOf(error);
    }
  }

  async function cancel(): Promise<void> {
    if (jobId === null) {
      return;
    }
    cancelling = true;
    try {
      await convertCancel(jobId);
      await poll(jobId);
    } catch (error) {
      running = false;
      cancelling = false;
      errorMessage = messageOf(error);
    }
  }

  async function chooseFile(): Promise<void> {
    errorMessage = '';
    try {
      const path = await convertPickFile();
      if (path !== null) {
        filePath = path;
        result = null;
      }
    } catch (error) {
      errorMessage = messageOf(error);
    }
  }

  onMount(() => {
    void loadHistory();
  });

  onDestroy(() => {
    if (timer !== undefined) {
      clearTimeout(timer);
      timer = undefined;
    }
  });
</script>

<main class="page">
  <div class="pagehead">
    <div class="titles">
      <h1>Video converter</h1>
      <div class="sub">Turn recordings into GIF, WebM or a smaller MP4</div>
    </div>
  </div>

  <div class="panel">
    <Segmented options={['History', 'File']} bind:value={mode} label="Video source" />

    {#if mode === 'History'}
      <section class="section" aria-label="Recordings in History">
        <p class="hint">Choose a recording from History.</p>
        {#if loadingHistory}
          <p class="hint">Reading History…</p>
        {:else if historyMessage}
          <p class="notice" role="status">{historyMessage}</p>
        {:else if captures.length === 0}
          <p class="hint">There are no recordings in History yet.</p>
        {:else}
          <div class="grid">
            {#each captures as capture (capture.id)}
              <button
                type="button"
                class="hv card"
                class:selected={capture.id === selectedId}
                aria-pressed={capture.id === selectedId}
                onclick={() => {
                  selectedId = capture.id;
                }}
              >
                <Thumb
                  id={capture.id}
                  alt=""
                  duration={capture.durationMs === null
                    ? undefined
                    : formatDuration(capture.durationMs)}
                />
                <span class="name">{capture.name}</span>
              </button>
            {/each}
          </div>
        {/if}
      </section>
    {:else}
      <section class="section" aria-label="Choose a file">
        <p class="hint">Choose a video file from this PC.</p>
        <div class="filerow">
          <Button icon="folder" onclick={chooseFile}>Choose video</Button>
          {#if filePath}
            <span class="path" title={filePath}>{filePath}</span>
          {:else}
            <span class="hint">No video chosen.</span>
          {/if}
        </div>
      </section>
    {/if}

    <SelectRow icon="film" title="Output format" options={formatOptions} bind:value={formatLabel} />

    <div class="actions">
      <Button
        variant="accent"
        icon="film"
        onclick={start}
        disabled={!ready || running}
        title={!ready
          ? mode === 'History'
            ? 'Choose a recording first.'
            : 'Choose a video file first.'
          : undefined}
      >
        Convert
      </Button>
      {#if running}
        <Button icon="stop" onclick={cancel} disabled={cancelling}>
          {cancelling ? 'Cancelling…' : 'Cancel'}
        </Button>
      {/if}
    </div>

    {#if running || progress}
      <div class="progressrow">
        <div class="progresshead" aria-live="polite">
          <span>{phaseLabel}</span>
          {#if rounded !== null}
            <span>{rounded}%</span>
          {/if}
        </div>
        <div
          class="bar"
          role="progressbar"
          aria-label="Conversion progress"
          aria-valuemin="0"
          aria-valuemax="100"
          aria-valuenow={rounded ?? undefined}
        >
          <div
            class="fill"
            class:indeterminate={rounded === null}
            style:width="{rounded ?? 0}%"
          ></div>
        </div>
      </div>
    {/if}

    {#if result}
      <div class="result" role="status">
        <Icon name="check" size={16} />
        <span>Saved {result.name}</span>
        <a href="#/history">Open in History</a>
      </div>
    {/if}

    {#if errorMessage}
      <p class="notice" role="alert">{errorMessage}</p>
    {/if}
  </div>
</main>

<style>
  .page {
    flex: 999 1 460px;
    min-width: 0;
    padding: 20px 24px 32px;
    display: flex;
    flex-direction: column;
    gap: 20px;
    overflow-y: auto;
  }

  .pagehead {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-end;
    justify-content: space-between;
    gap: 12px;
  }

  .titles {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  h1 {
    margin: 0;
    font-size: 28px;
    line-height: 36px;
    font-weight: 600;
  }

  .sub {
    font-size: 12px;
    color: var(--text3);
  }

  .panel {
    width: 100%;
    max-width: 760px;
    padding: 20px;
    background: var(--card);
    border: 1px solid var(--stroke);
    border-radius: 6px;
    display: flex;
    flex-direction: column;
    gap: 20px;
  }

  .section {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  .hint {
    margin: 0;
    font-size: 12px;
    line-height: 18px;
    color: var(--text3);
  }

  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(150px, 1fr));
    gap: 12px;
    max-height: 320px;
    overflow-y: auto;
  }

  .card {
    padding: 6px;
    background: var(--layer);
    border: 1px solid var(--stroke);
    border-radius: 6px;
    color: var(--text);
    text-align: left;
    display: flex;
    flex-direction: column;
    gap: 6px;
    min-width: 0;
  }

  .card.selected {
    border-color: var(--accent);
    box-shadow: 0 0 0 1px var(--accent);
  }

  .name {
    padding: 0 4px 4px;
    font-size: 12px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .filerow {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 12px;
  }

  .path {
    min-width: 0;
    font-family: var(--type-mono);
    font-size: 12px;
    color: var(--text2);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
  }

  .progressrow {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .progresshead {
    display: flex;
    justify-content: space-between;
    gap: 12px;
    font-size: 12px;
    color: var(--text2);
  }

  .bar {
    height: 6px;
    background: var(--subtle);
    border-radius: 3px;
    overflow: hidden;
  }

  .fill {
    height: 100%;
    background: var(--accent);
    transition: width 150ms linear;
  }

  .fill.indeterminate {
    width: 35% !important;
  }

  .result {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
    padding: 10px 12px;
    background: var(--layer);
    border: 1px solid var(--stroke);
    border-radius: 4px;
    font-size: 12px;
    color: var(--text2);
  }

  .result a {
    margin-left: auto;
    color: var(--accent);
  }

  .notice {
    margin: 0;
    padding: 10px 12px;
    border: 1px solid var(--err);
    border-radius: 4px;
    color: var(--err);
    font-size: 12px;
  }

  @media (prefers-reduced-motion: reduce) {
    .fill {
      transition: none;
    }
  }
</style>
