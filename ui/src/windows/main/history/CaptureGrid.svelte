<script lang="ts">
  import Badge from '../../../components/Badge.svelte';
  import Thumb from '../../../components/Thumb.svelte';
  import type { HistoryCaptureDto } from '../../../lib/ipc/bindings/HistoryCaptureDto';
  import { badgeStatus, formatBytes, formatDuration, formatTime, typeTag } from './data';

  interface Group {
    key: string;
    label: string;
    countLabel: string;
    items: HistoryCaptureDto[];
  }

  interface Props {
    groups: Group[];
    selectedId: number | null;
    columns: number;
    viewportWidth: number;
    viewportHeight: number;
    scrollTop?: number;
    scroller?: HTMLDivElement;
    onselect: (id: number) => void;
    onopen: (id: number) => void;
  }

  let {
    groups,
    selectedId,
    columns,
    viewportWidth,
    viewportHeight,
    scrollTop = $bindable(0),
    scroller,
    onselect,
    onopen,
  }: Props = $props();

  interface HeaderRow {
    key: string;
    kind: 'header';
    top: number;
    height: number;
    label: string;
    count: string;
  }

  interface CardsRow {
    key: string;
    kind: 'cards';
    top: number;
    height: number;
    items: HistoryCaptureDto[];
  }

  type Row = HeaderRow | CardsRow;

  const cardWidth = $derived((Math.max(viewportWidth, 320) - (columns - 1) * 12) / columns);
  const rowHeight = $derived(Math.round(cardWidth * 0.625) + 64 + 12);
  const height = $derived(Math.max(viewportHeight, 320));

  const layout = $derived.by(() => {
    const rows: Row[] = [];
    let top = 0;
    for (const group of groups) {
      rows.push({
        key: `header:${group.key}`,
        kind: 'header',
        top,
        height: 46,
        label: group.label,
        count: group.countLabel,
      });
      top += 46;
      for (let index = 0; index < group.items.length; index += columns) {
        rows.push({
          key: `cards:${group.key}:${index}`,
          kind: 'cards',
          top,
          height: rowHeight,
          items: group.items.slice(index, index + columns),
        });
        top += rowHeight;
      }
    }
    return { rows, total: top };
  });

  const visible = $derived.by(() => {
    const start = scrollTop - height;
    const end = scrollTop + height * 2;
    return layout.rows.filter((row) => row.top + row.height >= start && row.top <= end);
  });

  const topPad = $derived(visible.length > 0 ? visible[0].top : 0);
  const bottomPad = $derived.by(() => {
    if (visible.length === 0) {
      return layout.total;
    }
    const last = visible[visible.length - 1];
    return Math.max(0, layout.total - (last.top + last.height));
  });

  $effect(() => {
    const id = selectedId;
    if (id === null) {
      return;
    }
    const row = layout.rows.find(
      (candidate) =>
        candidate.kind === 'cards' && candidate.items.some((capture) => capture.id === id),
    );
    if (!row) {
      return;
    }
    const height = Math.max(viewportHeight, 320);
    if (row.top < scrollTop || row.top + row.height > scrollTop + height) {
      const target = Math.max(0, row.top - 12);
      scrollTop = target;
      if (scroller) {
        scroller.scrollTop = target;
      }
    }
  });

  function duration(capture: HistoryCaptureDto): string | undefined {
    return capture.durationMs === null ? undefined : formatDuration(capture.durationMs);
  }
</script>

<div style:height="{topPad}px"></div>
{#each visible as row (row.key)}
  {#if row.kind === 'header'}
    <h2 class="grouphead" style:height="{row.height}px">
      <span>{row.label}</span>
      <span class="count">· {row.count}</span>
    </h2>
  {:else}
    <div
      class="gridrow"
      style:height="{row.height - 12}px"
      style:grid-template-columns="repeat({columns}, minmax(0, 1fr))"
    >
      {#each row.items as capture (capture.id)}
        <button
          type="button"
          class="hv card"
          class:selected={capture.id === selectedId}
          aria-pressed={capture.id === selectedId}
          data-capture-id={capture.id}
          onclick={() => onselect(capture.id)}
          ondblclick={() => onopen(capture.id)}
        >
          <Thumb
            id={capture.id}
            alt=""
            duration={duration(capture)}
            tag={duration(capture) ? typeTag(capture.kind, capture.name) : undefined}
          />
          <div class="meta">
            <div class="name">{capture.name}</div>
            <div class="line">
              <span class="when"
                >{formatTime(capture.createdAt)} · {formatBytes(capture.bytes)}</span
              >
              <Badge status={badgeStatus(capture)} />
            </div>
          </div>
        </button>
      {/each}
    </div>
  {/if}
{/each}
<div style:height="{bottomPad}px"></div>

<style>
  .grouphead {
    margin: 0;
    display: flex;
    align-items: center;
    gap: 4px;
    font-size: 14px;
    font-weight: 600;
  }

  .count {
    font-weight: 400;
    color: var(--text3);
  }

  .gridrow {
    display: grid;
    gap: 12px;
    margin-bottom: 12px;
    align-items: stretch;
  }

  .card {
    background: var(--card);
    color: var(--text);
    border-radius: 6px;
    padding: 6px 6px 10px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    text-align: left;
    min-width: 0;
    height: 100%;
    overflow: hidden;
    border: 1px solid var(--stroke);
  }

  .card.selected {
    border-color: var(--accent);
    box-shadow: 0 0 0 1px var(--accent);
  }

  .meta {
    padding: 0 4px;
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
  }

  .name {
    font-size: 12px;
    font-weight: 500;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .line {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 6px;
    font-size: 11px;
    color: var(--text3);
  }

  .when {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
</style>
