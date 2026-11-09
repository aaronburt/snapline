<script lang="ts">
  import Badge from '../../../components/Badge.svelte';
  import Thumb from '../../../components/Thumb.svelte';
  import type { HistoryCaptureDto } from '../../../lib/ipc/bindings/HistoryCaptureDto';
  import {
    badgeStatus,
    formatBytes,
    formatCaptured,
    formatDuration,
    typeLabel,
  } from './data';

  interface Props {
    items: HistoryCaptureDto[];
    selectedId: number | null;
    viewportHeight: number;
    scrollTop?: number;
    scroller?: HTMLDivElement;
    onselect: (id: number) => void;
    onopen: (id: number) => void;
  }

  let {
    items,
    selectedId,
    viewportHeight,
    scrollTop = $bindable(0),
    scroller,
    onselect,
    onopen,
  }: Props = $props();

  const HEADER = 38;
  const ROW = 52;

  const offset = $derived(Math.max(0, scrollTop - HEADER));
  const first = $derived(Math.floor(offset / ROW));
  const overscan = $derived(Math.max(1, Math.ceil(Math.max(viewportHeight, 320) / ROW)));
  const start = $derived(Math.max(0, first - overscan));
  const end = $derived(Math.min(items.length, first + overscan * 2));
  const shown = $derived(items.slice(start, end));
  const topPad = $derived(start * ROW);
  const bottomPad = $derived(Math.max(0, (items.length - end) * ROW));

  $effect(() => {
    const id = selectedId;
    if (id === null) {
      return;
    }
    const index = items.findIndex((capture) => capture.id === id);
    if (index === -1) {
      return;
    }
    const rowTop = HEADER + index * ROW;
    const height = Math.max(viewportHeight, 320);
    if (rowTop < scrollTop || rowTop + ROW > scrollTop + height) {
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
        <th scope="col">Type</th>
        <th scope="col">Size</th>
        <th scope="col">Captured</th>
        <th scope="col">Status</th>
      </tr>
    </thead>
    <tbody>
      {#if topPad > 0}
        <tr class="spacer" aria-hidden="true">
          <td colspan="5" style:height="{topPad}px"></td>
        </tr>
      {/if}
      {#each shown as capture (capture.id)}
        <tr
          class:selected={capture.id === selectedId}
          aria-current={capture.id === selectedId ? 'true' : undefined}
          data-capture-id={capture.id}
          tabindex="0"
          onclick={() => onselect(capture.id)}
          onkeydown={(event) => {
            if (event.key === 'Enter' || event.key === ' ') {
              event.preventDefault();
              onselect(capture.id);
            }
          }}
          ondblclick={() => onopen(capture.id)}
        >
          <td>
            <span class="file">
              <span class="thumb"><Thumb id={capture.id} alt={capture.name} /></span>
              <span class="filename">{capture.name}</span>
            </span>
          </td>
          <td class="muted">{typeLabel(capture.kind, capture.name)}</td>
          <td class="muted">{formatBytes(capture.bytes)}</td>
          <td class="muted">
            {formatCaptured(capture.createdAt)}
            {#if capture.durationMs !== null}
              <span class="duration">· {formatDuration(capture.durationMs)}</span>
            {/if}
          </td>
          <td><Badge status={badgeStatus(capture)} /></td>
        </tr>
      {/each}
      {#if bottomPad > 0}
        <tr class="spacer" aria-hidden="true">
          <td colspan="5" style:height="{bottomPad}px"></td>
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
    overflow-x: auto;
  }

  table {
    width: 100%;
    border-collapse: collapse;
    min-width: 720px;
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
    height: 52px;
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

  .filename {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .muted {
    color: var(--text2);
    white-space: nowrap;
  }

  .duration {
    color: var(--text3);
  }
</style>
