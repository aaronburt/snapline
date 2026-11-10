<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import { onEvent } from '../../../lib/events';
  import EmptyState from '../../../components/EmptyState.svelte';
  import Icon from '../../../components/Icon.svelte';
  import Segmented from '../../../components/Segmented.svelte';
  import type { Cursor } from '../../../lib/ipc/bindings/Cursor';
  import type { HistoryCaptureDto } from '../../../lib/ipc/bindings/HistoryCaptureDto';
  import type { HistoryStats } from '../../../lib/ipc/bindings/HistoryStats';
  import type { UploadState } from '../../../lib/ipc/bindings/UploadState';
  import {
    historyConfirmDelete,
    historyCopyImage,
    historyDelete,
    historyOpenFile,
    historyPage,
    historySearch,
    historyShowInFolder,
    historyStats,
  } from '../../../lib/ipc/history';
  import { get as settingsGet } from '../../../lib/ipc/settings';
  import { cancelUpload, listUploadStates, pickAndUpload } from '../../../lib/ipc/upload';
  import { editorOpenHistory, pinHistory } from '../../../lib/ipc/tool';
  import CaptureGrid from './CaptureGrid.svelte';
  import CaptureList from './CaptureList.svelte';
  import DetailPane from './DetailPane.svelte';
  import { captureCardGap, captureCardWidth, filterValue, formatBytes, groupCaptures, historyFilters } from './data';
  import { searchQuery } from './search';

  let filterLabel = $state('All');
  let view = $state<'grid' | 'list'>('grid');
  let items = $state<HistoryCaptureDto[]>([]);
  let next = $state<Cursor | null>(null);
  let loading = $state(false);
  let loaded = $state(false);
  let errorMessage = $state('');
  let selectedId = $state<number | null>(null);
  let scroller = $state<HTMLDivElement | undefined>(undefined);
  let scrollTop = $state(0);
  let viewportHeight = $state(600);
  let viewportWidth = $state(800);
  let defaultDestination = $state('Save locally only');
  let uploadsById = $state<Record<number, UploadState>>({});
  let stats = $state<HistoryStats>({ count: 0, bytes: 0, uploadedThisWeek: 0 });
  let token = 0;

  const filter = $derived(filterValue(filterLabel));
  const searching = $derived($searchQuery.trim() !== '');
  const groups = $derived(groupCaptures(items));
  const selected = $derived(items.find((capture) => capture.id === selectedId) ?? null);
  const activeUploads = $derived.by(() => {
    const byCapture: Record<number, UploadState> = {};
    for (const upload of Object.values(uploadsById)) {
      if (upload.captureId === null) continue;
      const previous = byCapture[upload.captureId];
      if (!previous || (previous.status === 'queued' && upload.status !== 'queued')) {
        byCapture[upload.captureId] = upload;
      }
    }
    return byCapture;
  });
  const columns = $derived(Math.max(1, Math.floor((viewportWidth + captureCardGap) / (captureCardWidth + captureCardGap))));

  function describe(error: unknown): string {
    if (
      error !== null &&
      typeof error === 'object' &&
      'message' in error &&
      typeof (error as { message: unknown }).message === 'string'
    ) {
      return (error as { message: string }).message;
    }
    return 'The History could not be read.';
  }

  async function fetchPage(reset: boolean): Promise<void> {
    const current = ++token;
    const cursor = reset ? null : next;
    const minimum = reset ? items.length : 0;
    loading = true;
    try {
      const term = $searchQuery.trim();
      const read = (at: typeof cursor) =>
        term ? historySearch(term, filter, at) : historyPage(filter, at);
      const page = await read(cursor);
      if (reset) {
        while (page.next && page.items.length < minimum && current === token) {
          const more = await read(page.next);
          page.items.push(...more.items);
          page.next = more.next;
        }
      }
      if (current !== token) {
        return;
      }
      if (reset) {
        items = page.items;
        selectedId = page.items.some((item) => item.id === selectedId) ? selectedId : null;
      } else {
        const known = new Set(items.map((capture) => capture.id));
        items = [...items, ...page.items.filter((capture) => !known.has(capture.id))];
      }
      next = page.next;
      loaded = true;
      errorMessage = '';
    } catch (error) {
      if (current !== token) {
        return;
      }
      loaded = true;
      errorMessage = describe(error);
    } finally {
      if (current === token) {
        loading = false;
      }
    }
  }

  async function refreshStats(): Promise<void> {
    try {
      stats = await historyStats();
    } catch {
      stats = { count: 0, bytes: 0, uploadedThisWeek: 0 };
    }
  }

  function reload(): void {
    items = [];
    selectedId = null;
    next = null;
    loaded = false;
    scrollTop = 0;
    if (scroller) {
      scroller.scrollTop = 0;
    }
    void fetchPage(true);
    void refreshStats();
  }

  $effect(() => {
    const term = $searchQuery;
    const label = filterLabel;
    void term;
    void label;
    untrack(() => {
      reload();
    });
  });

  onMount(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    let observed: Set<number> | null = new Set();
    void onEvent((event) => {
      if (event.area === 'upload' && event.event.kind === 'stateChanged') {
        const state = event.event.data;
        if (state.captureId !== null) {
          observed?.add(state.id);
          if (['done', 'failed', 'cancelled'].includes(state.status)) {
            delete uploadsById[state.id];
          } else {
            uploadsById[state.id] = state;
          }
        }
      }
      if ((event.area === 'history' && event.event.kind !== 'ping') ||
          (event.area === 'upload' && event.event.kind === 'stateChanged' &&
           ['queued', 'done', 'failed', 'cancelled'].includes(event.event.data.status))) {
        void fetchPage(true);
        void refreshStats();
      }
    }).then((stop) => {
      if (disposed) stop();
      else unlisten = stop;
    }).catch(() => undefined).then(async () => {
      if (disposed) return;
      const states = await listUploadStates();
      if (disposed) return;
      for (const state of states) {
        if (state.captureId !== null && !observed?.has(state.id) &&
            !['done', 'failed', 'cancelled'].includes(state.status)) {
          uploadsById[state.id] = state;
        }
      }
    }).catch(() => undefined).finally(() => { observed = null; });
    void settingsGet()
      .then((settings) => {
        const destination = settings.uploads.images;
        if (destination) {
          defaultDestination = destination;
        }
      })
      .catch(() => undefined);
    return () => { disposed = true; unlisten?.(); };
  });

  function onscroll(event: Event): void {
    const element = event.currentTarget as HTMLDivElement;
    scrollTop = element.scrollTop;
    viewportHeight = element.clientHeight || viewportHeight;
    viewportWidth = element.clientWidth || viewportWidth;
    if (next && !loading) {
      const nearBottom = element.scrollTop + element.clientHeight >= element.scrollHeight - 480;
      if (nearBottom) {
        void fetchPage(false);
      }
    }
  }

  function select(id: number): void {
    selectedId = id;
  }

  function closeDetails(): void {
    const row = scroller?.querySelector<HTMLElement>(`[data-capture-id="${selectedId}"]`);
    selectedId = null;
    row?.focus({ preventScroll: true });
  }

  function deselectBackground(event: MouseEvent): void {
    const target = event.target;
    if (target instanceof Element && target.closest('main.page') && !target.closest('[data-capture-id], button, a, input, select, textarea')) {
      selectedId = null;
    }
  }

  function move(delta: number): void {
    if (items.length === 0) {
      return;
    }
    const index = items.findIndex((capture) => capture.id === selectedId);
    if (index === -1) {
      selectedId = items[0].id;
      return;
    }
    const step = view === 'grid' ? delta * columns : delta;
    const target = Math.min(items.length - 1, Math.max(0, index + step));
    selectedId = items[target].id;
  }

  function copyImage(id: number): void {
    void historyCopyImage(id).catch((error: unknown) => {
      errorMessage = describe(error);
    });
  }

  function showInFolder(id: number): void {
    void historyShowInFolder(id).catch((error: unknown) => {
      errorMessage = describe(error);
    });
  }

  function openFile(id: number): void {
    void historyOpenFile(id).catch((error: unknown) => {
      errorMessage = describe(error);
    });
  }

  function annotate(id: number): void {
    void editorOpenHistory(id)
      .then((outcome) => {
        if (!outcome.cancelled && outcome.captureId !== null) {
          void fetchPage(true);
          void refreshStats();
        }
      })
      .catch((error: unknown) => {
        errorMessage = describe(error);
      });
  }

  function pin(id: number): void {
    void pinHistory(id).catch((error: unknown) => { errorMessage = describe(error); });
  }

  async function uploadFiles(): Promise<void> {
    try {
      const ids = await pickAndUpload();
      if (ids.length > 0) {
        await fetchPage(true);
        await refreshStats();
      }
    } catch (error) {
      errorMessage = describe(error);
    }
  }

  function cancelRowUpload(id: number): void {
    const upload = activeUploads[id];
    if (!upload) {
      return;
    }
    void cancelUpload(upload.id).catch((error: unknown) => {
      errorMessage = describe(error);
    });
  }

  async function askDelete(id: number): Promise<void> {
    try {
      const confirmed = await historyConfirmDelete(id);
      if (!confirmed) {
        return;
      }
      await historyDelete(id);
      items = items.filter((capture) => capture.id !== id);
      if (selectedId === id) {
        selectedId = null;
      }
      void refreshStats();
    } catch (error) {
      errorMessage = describe(error);
    }
  }

  function openSelected(): void {
    if (selectedId !== null) {
      openFile(selectedId);
    }
  }

  function copySelected(): void {
    if (selectedId !== null) {
      copyImage(selectedId);
    }
  }

  function onkeydown(event: KeyboardEvent): void {
    if (event.defaultPrevented) {
      return;
    }
    const target = event.target as HTMLElement | null;
    if (target && ['INPUT', 'SELECT', 'TEXTAREA'].includes(target.tagName)) {
      return;
    }
    const focusable =
      target && typeof target.closest === 'function'
        ? target.closest('button, a, [tabindex]')
        : null;
    if (focusable && (event.key === 'Enter' || event.key === ' ')) {
      return;
    }
    if (event.key === 'ArrowRight') {
      event.preventDefault();
      move(1);
    } else if (event.key === 'ArrowLeft') {
      event.preventDefault();
      move(-1);
    } else if (event.key === 'ArrowDown') {
      event.preventDefault();
      move(1);
    } else if (event.key === 'ArrowUp') {
      event.preventDefault();
      move(-1);
    } else if (event.key === 'Enter') {
      event.preventDefault();
      openSelected();
    } else if (event.key === 'Delete') {
      event.preventDefault();
      if (selectedId !== null) {
        void askDelete(selectedId);
      }
    } else if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'c') {
      event.preventDefault();
      copySelected();
    }
  }
</script>

<svelte:window {onkeydown} onclick={deselectBackground} />

<div class="history-layout">
<main class="page">
  <div class="pagehead">
    <div class="titles">
      <h1>History</h1>
      <div class="sub" aria-live="polite">
        {stats.count} captures · {formatBytes(stats.bytes)} on disk · {stats.uploadedThisWeek} uploaded
        this week
      </div>
    </div>
    <div class="controls">
      <button type="button" class="upload-file hv" onclick={() => void uploadFiles()}>
        Upload file…
      </button>
      <Segmented
        options={historyFilters}
        value={filterLabel}
        label="Filter by type"
        onchange={(value) => {
          filterLabel = value;
        }}
      />
      <div class="view-options">
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
  </div>

  {#if errorMessage}
    <p class="error" role="status">{errorMessage}</p>
  {/if}

  <div
    class="scroll"
    bind:this={scroller}
    bind:clientWidth={viewportWidth}
    bind:clientHeight={viewportHeight}
    {onscroll}
  >
    {#if loaded && !loading && !errorMessage && items.length === 0}
      <div class="empty">
        {#if searching || filter !== 'all'}
          <EmptyState title="No matching items" text="Clear the search or change the filter to see more of your history." />
          <div class="empty-actions">
            {#if searching}
              <button type="button" class="upload-file hv" onclick={() => searchQuery.set('')}>Clear search</button>
            {/if}
            {#if filter !== 'all'}
              <button type="button" class="upload-file hv" onclick={() => (filterLabel = 'All')}>Reset filters</button>
            {/if}
          </div>
        {:else}
          <EmptyState title="No captures yet" text="Press PrtSc to capture a region." />
          <button type="button" class="upload-file primary hv" onclick={() => void uploadFiles()}>Upload file…</button>
        {/if}
      </div>
    {:else if view === 'grid'}
      <CaptureGrid
        {groups}
        {activeUploads}
        {selectedId}
        {columns}
        {viewportHeight}
        bind:scrollTop
        {scroller}
        onselect={select}
        onopen={openFile}
      />
    {:else}
      <CaptureList
        {items}
        {activeUploads}
        {selectedId}
        {viewportHeight}
        {viewportWidth}
        bind:scrollTop
        {scroller}
        onselect={select}
        onopen={openFile}
        oncancel={cancelRowUpload}
      />
    {/if}
    {#if loading}
      <div class="loading" role="status"><span class="sr-only">Loading captures</span></div>
    {/if}
  </div>
</main>

{#if selected}
<DetailPane
  capture={selected}
  activeUpload={selected ? activeUploads[selected.id] ?? null : null}
  {defaultDestination}
  oncopy={copyImage}
  onshow={showInFolder}
  onopen={openFile}
  onannotate={annotate}
  onpin={pin}
  onclose={closeDetails}
  ondelete={(id) => {
    void askDelete(id);
  }}
/>
{/if}
</div>

<style>
  .history-layout {
    display: flex;
    flex: 1;
    min-width: 0;
    min-height: 0;
  }

  @media (max-width: 1050px) {
    .history-layout {
      flex-direction: column;
    }

    .history-layout .page {
      flex: 1 1 0;
    }
  }

  .page {
    flex: 999 1 460px;
    min-width: 0;
    min-height: 0;
    padding: 20px 24px 0;
    display: flex;
    flex-direction: column;
    gap: 16px;
    overflow: hidden;
  }

  .pagehead {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-end;
    justify-content: space-between;
    gap: 12px;
    flex: none;
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
    flex-wrap: wrap;
    max-width: 100%;
    align-items: center;
    gap: 8px;
  }

  .view-options,
  .empty-actions {
    display: flex;
    gap: 8px;
  }

  .upload-file {
    min-height: 34px;
    padding: 0 10px;
    border: 1px solid var(--stroke);
    border-radius: 4px;
    background: var(--card);
    color: var(--text);
    font-size: 12px;
    white-space: nowrap;
  }

  .upload-file.primary {
    background: var(--accent);
    color: var(--onAccent);
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

  .scroll {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding-bottom: 32px;
  }

  .empty {
    height: 100%;
    display: flex;
    flex-direction: column;
    gap: 14px;
    align-items: center;
    justify-content: center;
  }

  .error {
    margin: 0;
    font-size: 12px;
    color: var(--err);
  }

  .loading {
    height: 24px;
  }
</style>
