<script lang="ts">
  import EmptyState from '../../../components/EmptyState.svelte';
  import Icon from '../../../components/Icon.svelte';
  import Segmented from '../../../components/Segmented.svelte';
  import CaptureGrid from './CaptureGrid.svelte';
  import CaptureList from './CaptureList.svelte';
  import DetailPane from './DetailPane.svelte';
  import { searchQuery } from './search';
  import {
    captures,
    captureCount,
    diskSize,
    filterCaptures,
    groupCaptures,
    historyFilters,
    moveSelection,
    uploadedThisWeek,
  } from './data';
  import type { HistoryFilter } from './data';

  let filter = $state<HistoryFilter>('All');
  let view = $state<'grid' | 'list'>('grid');
  let selectedId = $state<number | null>(1);

  const shown = $derived(filterCaptures(captures, filter, $searchQuery));
  const groups = $derived(groupCaptures(shown));
  const selected = $derived(shown.find((capture) => capture.id === selectedId) ?? null);

  function openSelected(id?: number): void {
    const target = id ?? selectedId;
    if (target !== null) {
      selectedId = target;
    }
  }

  function copySelected(): void {
    void selected;
  }

  function askDelete(): void {
    if (!selected) {
      return;
    }
    if (window.confirm(`Delete ${selected.name}? The file goes to the Recycle Bin.`)) {
      selectedId = null;
    }
  }

  function onkeydown(event: KeyboardEvent): void {
    const target = event.target as HTMLElement | null;
    if (target && ['INPUT', 'SELECT', 'TEXTAREA'].includes(target.tagName)) {
      return;
    }
    if (
      target?.closest('button, a, [tabindex]') &&
      (event.key === 'Enter' || event.key === ' ')
    ) {
      return;
    }
    if (event.key === 'ArrowRight' || event.key === 'ArrowDown') {
      event.preventDefault();
      selectedId = moveSelection(shown, selectedId, 1);
    } else if (event.key === 'ArrowLeft' || event.key === 'ArrowUp') {
      event.preventDefault();
      selectedId = moveSelection(shown, selectedId, -1);
    } else if (event.key === 'Enter') {
      event.preventDefault();
      openSelected();
    } else if (event.key === 'Delete') {
      event.preventDefault();
      askDelete();
    } else if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'c') {
      event.preventDefault();
      copySelected();
    }
  }
</script>

<svelte:window onkeydown={onkeydown} />

<main class="page">
  <div class="pagehead">
    <div class="titles">
      <h1>History</h1>
      <div class="sub">
        {captureCount} captures · {diskSize} on disk · {uploadedThisWeek} uploaded this week
      </div>
    </div>
    <div class="controls">
      <Segmented
        options={historyFilters}
        value={filter}
        label="Filter by type"
        onchange={(value) => {
          filter = value as HistoryFilter;
        }}
      />
      <button
        type="button"
        class="hv viewbtn"
        class:on={view === 'grid'}
        aria-label="Grid view"
        aria-pressed={view === 'grid'}
        onclick={() => {
          view = 'grid';
        }}
      >
        <Icon name="grid" size={16} />
      </button>
      <button
        type="button"
        class="hv viewbtn"
        class:on={view === 'list'}
        aria-label="List view"
        aria-pressed={view === 'list'}
        onclick={() => {
          view = 'list';
        }}
      >
        <Icon name="list" size={16} strokeWidth={1.8} />
      </button>
    </div>
  </div>

  {#if shown.length === 0}
    <div class="empty">
      <EmptyState title="No captures yet" text="Press PrtSc to capture a region." />
    </div>
  {:else if view === 'grid'}
    <CaptureGrid
      {groups}
      {selectedId}
      onselect={(id) => {
        selectedId = id;
      }}
      onopen={openSelected}
    />
  {:else}
    <CaptureList
      items={shown}
      {selectedId}
      onselect={(id) => {
        selectedId = id;
      }}
      onopen={openSelected}
    />
  {/if}
</main>

<DetailPane capture={selected} />

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

  .controls {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .viewbtn {
    width: 34px;
    height: 34px;
    border: 1px solid transparent;
    background: transparent;
    color: var(--text2);
    border-radius: 4px;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .viewbtn.on {
    border-color: var(--stroke);
    background: var(--card);
    color: var(--text);
  }

  .empty {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
  }
</style>
