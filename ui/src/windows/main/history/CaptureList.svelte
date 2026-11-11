<script lang="ts">
  import { untrack } from 'svelte';
  import Badge from '../../../components/Badge.svelte';
  import Thumb from '../../../components/Thumb.svelte';
  import UploadProgress from '../../../components/UploadProgress.svelte';
  import type { HistoryCaptureDto } from '../../../lib/ipc/bindings/HistoryCaptureDto';
  import type { UploadState } from '../../../lib/ipc/bindings/UploadState';
  import {
    badgeStatus,
    formatBytes,
    formatCaptured,
    formatDuration,
    pendingUploadLabel,
    typeLabel,
    typeTag,
  } from './data';

  interface Props {
    items: HistoryCaptureDto[];
    activeUploads?: Record<number, UploadState>;
    selectedIds: ReadonlySet<number>;
    focusId: number | null;
    viewportHeight: number;
    viewportWidth?: number;
    scrollTop?: number;
    scroller?: HTMLDivElement;
    onselect: (id: number, event: MouseEvent | KeyboardEvent) => void;
    onopen: (id: number) => void;
    oncancel?: (id: number) => void;
  }

  let {
    items,
    activeUploads = {},
    selectedIds,
    focusId,
    viewportHeight,
    viewportWidth = 800,
    scrollTop = $bindable(0),
    scroller,
    onselect,
    onopen,
    oncancel,
  }: Props = $props();

  const HEADER = 38;
  const ROW = 72;
  const compact = $derived(viewportWidth < 740);
  const inlineSize = $derived(viewportWidth < 460);
  const columnCount = $derived(compact ? (inlineSize ? 2 : 3) : 5);

  const offset = $derived(Math.max(0, scrollTop - HEADER));
  const first = $derived(Math.floor(offset / ROW));
  const overscan = $derived(Math.max(1, Math.ceil(Math.max(viewportHeight, 320) / ROW)));
  const start = $derived(Math.max(0, first - overscan));
  const end = $derived(Math.min(items.length, first + overscan * 2));
  const shown = $derived(items.slice(start, end));
  const topPad = $derived(start * ROW);
  const bottomPad = $derived(Math.max(0, (items.length - end) * ROW));

  $effect(() => {
    const id = focusId;
    if (id === null) {
      return;
    }
    const index = items.findIndex((capture) => capture.id === id);
    if (index === -1) {
      return;
    }
    const rowTop = HEADER + index * ROW;
    const height = viewportHeight || 320;
    const currentScrollTop = untrack(() => scrollTop);
    if (rowTop < currentScrollTop || rowTop + ROW > currentScrollTop + height) {
      const target = Math.max(0, rowTop - 12);
      scrollTop = target;
      if (scroller) {
        scroller.scrollTop = target;
      }
    }
  });
</script>

<div class="tablecard">
  <table>
    <caption class="sr-only">Captures</caption>
    <thead>
      <tr>
        <th scope="col">File</th>
        {#if !compact}<th scope="col" class="type-col">Type</th>{/if}
        {#if !inlineSize}<th scope="col" class="size-col">Size</th>{/if}
        {#if !compact}<th scope="col" class="captured-col">Captured</th>{/if}
        <th scope="col" class="status-col">Status</th>
      </tr>
    </thead>
    <tbody>
      {#if topPad > 0}
        <tr class="spacer" aria-hidden="true">
          <td colspan={columnCount} style:height="{topPad}px"></td>
        </tr>
      {/if}
      {#each shown as capture (capture.id)}
        {@const active = activeUploads[capture.id]}
        {@const phase = active?.status ?? capture.upload?.state}
        {@const type = typeLabel(capture.kind, capture.name)}
        {@const captured = formatCaptured(capture.createdAt)}
        {@const duration = capture.durationMs !== null ? formatDuration(capture.durationMs) : null}
        <tr
          class:selected={selectedIds.has(capture.id)}
          data-capture-id={capture.id}
          tabindex="0"
          onclick={(event) => onselect(capture.id, event)}
          onkeydown={(event) => {
            if (event.target instanceof HTMLElement && event.target.closest('button')) {
              return;
            }
            if (event.key === 'Enter' || event.key === ' ') {
              event.preventDefault();
              onselect(capture.id, event);
            }
          }}
          ondblclick={(event) => {
            if (!event.ctrlKey && !event.metaKey && !event.shiftKey) onopen(capture.id);
          }}
        >
          <td>
            <span class="file">
              {#if capture.kind === 'text' || capture.kind === 'file'}
                <span class="thumb file-type" aria-hidden="true"
                  >{typeTag(capture.kind, capture.name)}</span
                >
              {:else}
                <span class="thumb"><Thumb id={capture.id} alt="" /></span>
              {/if}
              <span class="file-label">
                <span class="filename" title={capture.name}>{capture.name}</span>
                {#if selectedIds.has(capture.id)}<span class="sr-only">Selected</span>{/if}
                {#if compact}
                  <span class="metadata">
                    {type} · {captured}{#if duration}
                      · {duration}{/if}{#if inlineSize}
                      · {formatBytes(capture.bytes)}{/if}
                  </span>
                {/if}
              </span>
            </span>
          </td>
          {#if !compact}<td class="muted" title={type}>{type}</td>{/if}
          {#if !inlineSize}<td class="muted">{formatBytes(capture.bytes)}</td>{/if}
          {#if !compact}
            <td class="muted" title={duration ? `${captured} · ${duration}` : captured}>
              {captured}
              {#if duration}<span class="duration">· {duration}</span>{/if}
            </td>
          {/if}
          <td>
            <div class="upload-status">
              <Badge
                status={active ? 'uploading' : badgeStatus(capture)}
                label={pendingUploadLabel(phase)}
              />
              {#if phase === 'uploading' || (active?.cancellable && oncancel)}
                <div class="progress-row">
                  {#if phase === 'uploading'}
                    <div class="progress-meter">
                      <UploadProgress
                        determinate={active?.progressAvailable}
                        sent={active?.sent}
                        total={active?.total}
                      />
                    </div>
                  {/if}
                  {#if active?.cancellable && oncancel}
                    <button
                      type="button"
                      class="cancel hv"
                      aria-label={`Cancel upload of ${capture.name}`}
                      onclick={(event) => {
                        event.stopPropagation();
                        oncancel?.(capture.id);
                      }}>Cancel</button
                    >
                  {/if}
                </div>
              {/if}
            </div>
          </td>
        </tr>
      {/each}
      {#if bottomPad > 0}
        <tr class="spacer" aria-hidden="true">
          <td colspan={columnCount} style:height="{bottomPad}px"></td>
        </tr>
      {/if}
    </tbody>
  </table>
</div>

<style>
  .tablecard {
    background: var(--card);
    border: 1px solid var(--stroke);
    border-radius: 6px;
  }

  table {
    width: 100%;
    border-collapse: collapse;
    table-layout: fixed;
  }

  .type-col {
    width: 108px;
  }
  .size-col {
    width: 84px;
  }
  .captured-col {
    width: 162px;
  }
  .status-col {
    width: 148px;
  }

  thead th {
    position: sticky;
    top: 0;
    z-index: 1;
    background: var(--card);
  }

  th {
    text-align: left;
    height: 38px;
    padding: 0 16px;
    font-size: 12px;
    font-weight: 400;
    color: var(--text3);
    border-bottom: 1px solid var(--stroke);
  }

  td {
    height: 72px;
    padding: 0 16px;
    font-size: 12px;
    border-top: 1px solid var(--stroke);
  }

  thead tr:first-child th {
    border-top: none;
  }

  tbody tr.spacer td {
    height: auto;
    padding: 0;
    border: none;
  }

  tbody tr {
    cursor: pointer;
  }

  tbody tr:hover {
    box-shadow: inset 0 0 0 999px var(--subtle);
  }

  tbody tr.selected {
    background: var(--accentSoft);
    box-shadow: inset 0 0 0 1px var(--accent);
  }

  .file {
    display: flex;
    align-items: center;
    gap: 10px;
    min-width: 0;
  }

  .thumb {
    width: 40px;
    flex: none;
    display: block;
  }

  .file-label {
    display: grid;
    min-width: 0;
    gap: 4px;
  }

  .file-type {
    display: grid;
    place-items: center;
    height: 25px;
    border: 1px solid var(--stroke);
    border-radius: 4px;
    background: var(--subtle);
    color: var(--text2);
    font-size: 10px;
  }

  .filename,
  .metadata,
  .muted {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .metadata {
    color: var(--text3);
    font-size: 11px;
  }

  .muted {
    color: var(--text2);
    white-space: nowrap;
  }

  .duration {
    color: var(--text3);
  }

  .upload-status {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 4px;
  }

  .progress-row {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
  }

  .progress-meter {
    flex: 1;
    min-width: 0;
  }

  .cancel {
    flex: none;
    min-height: 24px;
    padding: 0 6px;
    border: 1px solid var(--strokeHi);
    border-radius: 4px;
    background: var(--layer);
    color: var(--text);
    font-size: 11px;
  }
</style>
