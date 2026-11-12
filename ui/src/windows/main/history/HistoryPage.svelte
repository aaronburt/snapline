<script lang="ts">
  import { onMount, tick, untrack } from 'svelte';
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
    historyDeleteMany,
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
  import ResizeDivider from '../ResizeDivider.svelte';
  import {
    captureCardGap,
    captureCardWidth,
    filterValue,
    formatBytes,
    groupCaptures,
    historyFilters,
  } from './data';
  import { readMarquee, startMarquee, type Marquee } from './marquee';
  import { searchQuery } from './search';

  let filterLabel = $state('All');
  let detailWidth = $state(320);
  let view = $state<'grid' | 'list'>('grid');
  let items = $state<HistoryCaptureDto[]>([]);
  let next = $state<Cursor | null>(null);
  let loading = $state(false);
  let loaded = $state(false);
  let errorMessage = $state('');
  let selectedIds = $state.raw<ReadonlySet<number>>(new Set());
  let anchorId = $state<number | null>(null);
  let focusId = $state<number | null>(null);
  let deleting = $state(false);
  let deleteError = $state('');
  let marquee = $state.raw<ReturnType<typeof readMarquee> | null>(null);
  let scroller = $state<HTMLDivElement | undefined>(undefined);
  let scrollTop = $state(0);
  let viewportHeight = $state(600);
  let viewportWidth = $state(800);
  let defaultDestination = $state('Save locally only');
  let uploadsById = $state<Record<number, UploadState>>({});
  let stats = $state<HistoryStats>({ count: 0, bytes: 0, uploadedThisWeek: 0 });
  let token = 0;
  let drag: Marquee | null = null;
  let dragFrame = 0;
  let suppressClick = false;

  const filter = $derived(filterValue(filterLabel));
  const searching = $derived($searchQuery.trim() !== '');
  const groups = $derived(groupCaptures(items));
  const selected = $derived(
    selectedIds.size === 1 ? (items.find((capture) => selectedIds.has(capture.id)) ?? null) : null,
  );
  const highlighted = $derived(marquee?.ids ?? selectedIds);
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
  const columns = $derived(
    Math.max(1, Math.floor((viewportWidth + captureCardGap) / (captureCardWidth + captureCardGap))),
  );

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
        if (drag && items.some((item, index) => item.id !== page.items[index]?.id)) {
          stopDrag();
        }
        items = page.items;
        if (selectedIds.size || focusId !== null || anchorId !== null) {
          const available = new Set(items.map((item) => item.id));
          selectedIds = new Set([...selectedIds].filter((id) => available.has(id)));
          if (focusId !== null && !available.has(focusId)) focusId = null;
          if (anchorId !== null && !available.has(anchorId)) anchorId = null;
        }
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
    stopDrag();
    items = [];
    clearSelection();
    deleteError = '';
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
      if (
        (event.area === 'history' && event.event.kind !== 'ping') ||
        (event.area === 'upload' &&
          event.event.kind === 'stateChanged' &&
          ['queued', 'done', 'failed', 'cancelled'].includes(event.event.data.status))
      ) {
        void fetchPage(true);
        void refreshStats();
      }
    })
      .then((stop) => {
        if (disposed) stop();
        else unlisten = stop;
      })
      .catch(() => undefined)
      .then(async () => {
        if (disposed) return;
        const states = await listUploadStates();
        if (disposed) return;
        for (const state of states) {
          if (
            state.captureId !== null &&
            !observed?.has(state.id) &&
            !['done', 'failed', 'cancelled'].includes(state.status)
          ) {
            uploadsById[state.id] = state;
          }
        }
      })
      .catch(() => undefined)
      .finally(() => {
        observed = null;
      });
    void settingsGet()
      .then((settings) => {
        const destination = settings.uploads.images;
        if (destination) {
          defaultDestination = destination;
        }
      })
      .catch(() => undefined);
    return () => {
      disposed = true;
      unlisten?.();
      stopDrag();
    };
  });

  function onscroll(event: Event): void {
    const element = event.currentTarget as HTMLDivElement;
    scrollTop = element.scrollTop;
    viewportHeight = element.clientHeight || viewportHeight;
    viewportWidth = element.clientWidth || viewportWidth;
    if (drag?.active) scheduleMarquee();
    if (next && !loading) {
      const nearBottom = element.scrollTop + element.clientHeight >= element.scrollHeight - 480;
      if (nearBottom) {
        void fetchPage(false);
      }
    }
  }

  function clearSelection(): void {
    selectedIds = new Set();
    anchorId = null;
    focusId = null;
  }

  function select(id: number, event: MouseEvent | KeyboardEvent): void {
    const additive = event.ctrlKey || event.metaKey;
    const ids = additive ? new Set(selectedIds) : new Set<number>();
    if (event.shiftKey && anchorId !== null) {
      const from = items.findIndex((item) => item.id === anchorId);
      const to = items.findIndex((item) => item.id === id);
      if (from >= 0 && to >= 0) {
        for (const item of items.slice(Math.min(from, to), Math.max(from, to) + 1)) {
          ids.add(item.id);
        }
      }
    } else {
      anchorId = id;
      if (ids.has(id)) ids.delete(id);
      else ids.add(id);
    }
    selectedIds = ids;
    focusId = id;
  }

  function closeDetails(): void {
    const row = scroller?.querySelector<HTMLElement>(`[data-capture-id="${selected?.id}"]`);
    clearSelection();
    row?.focus({ preventScroll: true });
  }

  function deselectBackground(event: MouseEvent): void {
    const target = event.target;
    if (
      target instanceof Element &&
      target.closest('main.page') &&
      !target.closest('[data-capture-id], .selection-bar, button, a, input, select, textarea')
    ) {
      clearSelection();
    }
  }

  function move(delta: number, event: KeyboardEvent): void {
    if (items.length === 0) {
      return;
    }
    const index = items.findIndex((capture) => capture.id === (focusId ?? anchorId));
    const target = index < 0 ? 0 : Math.min(items.length - 1, Math.max(0, index + delta));
    const id = items[target].id;
    select(id, event);
    void tick().then(() => {
      scroller
        ?.querySelector<HTMLElement>(`[data-capture-id="${id}"]`)
        ?.focus({ preventScroll: true });
    });
  }

  function beginDrag(event: PointerEvent): void {
    if (event.button !== 0 || event.pointerType !== 'mouse') return;
    suppressClick = false;
    if (
      !scroller ||
      !items.length ||
      !(event.target instanceof Element) ||
      event.target.closest('button:not([data-capture-id]), a, input, select, textarea, th')
    ) {
      return;
    }
    const base = event.ctrlKey || event.metaKey || event.shiftKey ? selectedIds : new Set<number>();
    drag = startMarquee(scroller, event, base);
  }

  function scheduleMarquee(): void {
    if (!dragFrame) dragFrame = requestAnimationFrame(paintMarquee);
  }

  function paintMarquee(): void {
    dragFrame = 0;
    if (!scroller || !drag?.active) return;
    marquee = readMarquee(scroller, drag);
    if (marquee.scroll) {
      const before = scroller.scrollTop;
      scroller.scrollTop += marquee.scroll;
      if (scroller.scrollTop !== before) scheduleMarquee();
    }
  }

  function moveDrag(event: PointerEvent): void {
    if (!scroller || !drag || event.pointerId !== drag.pointerId) return;
    drag.x = event.clientX;
    drag.y = event.clientY;
    if (!drag.active) {
      if (Math.hypot(drag.x - drag.startX, drag.y - drag.startY) < 5) return;
      drag.active = true;
      suppressClick = true;
      scroller.setPointerCapture?.(drag.pointerId);
    }
    scheduleMarquee();
  }

  function finishDrag(event: PointerEvent): void {
    if (!scroller || !drag || event.pointerId !== drag.pointerId) return;
    if (drag.active) {
      drag.x = event.clientX;
      drag.y = event.clientY;
      const { ids } = readMarquee(scroller, drag);
      selectedIds = ids;
      anchorId = items.find((item) => ids.has(item.id))?.id ?? null;
      focusId = null;
    }
    stopDrag();
  }

  function stopDrag(): void {
    const pointerId = drag?.pointerId;
    drag = null;
    if (dragFrame) cancelAnimationFrame(dragFrame);
    dragFrame = 0;
    marquee = null;
    if (pointerId !== undefined && scroller?.hasPointerCapture?.(pointerId)) {
      scroller.releasePointerCapture(pointerId);
    }
  }

  function cancelDrag(event: PointerEvent): void {
    if (event.pointerId === drag?.pointerId) stopDrag();
  }

  function suppressDragClick(event: MouseEvent): void {
    if (suppressClick && event.detail > 0) {
      suppressClick = false;
      event.preventDefault();
      event.stopPropagation();
    }
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
    void pinHistory(id).catch((error: unknown) => {
      errorMessage = describe(error);
    });
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

  async function askDelete(ids: number[]): Promise<void> {
    if (deleting || !ids.length) return;
    deleting = true;
    deleteError = '';
    try {
      let removed = ids;
      if (ids.length === 1) {
        if (!(await historyConfirmDelete(ids[0]))) return;
        await historyDelete(ids[0]);
      } else {
        const result = await historyDeleteMany(ids);
        removed = result.removed;
        if (result.failed) {
          const noun = result.failed === 1 ? 'item' : 'items';
          deleteError = `Could not delete ${result.failed} ${noun}. Try again.`;
        }
      }
      if (removed.length) {
        const deleted = new Set(removed);
        items = items.filter((capture) => !deleted.has(capture.id));
        selectedIds = new Set([...selectedIds].filter((id) => !deleted.has(id)));
        if (focusId !== null && deleted.has(focusId)) focusId = null;
        if (anchorId !== null && deleted.has(anchorId)) anchorId = null;
        void refreshStats();
      }
    } catch (error) {
      deleteError = describe(error);
    } finally {
      deleting = false;
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
    if (
      target instanceof Element &&
      target !== document.body &&
      target !== document.documentElement &&
      !target.closest('.scroll, .selection-bar')
    ) {
      return;
    }
    const focusable =
      target && typeof target.closest === 'function'
        ? target.closest('button, a, [tabindex]')
        : null;
    if (focusable && (event.key === 'Enter' || event.key === ' ')) {
      return;
    }
    if (event.key === 'Escape' && (drag || selectedIds.size)) {
      event.preventDefault();
      if (drag) stopDrag();
      else clearSelection();
    } else if (event.key === 'ArrowRight') {
      event.preventDefault();
      move(1, event);
    } else if (event.key === 'ArrowLeft') {
      event.preventDefault();
      move(-1, event);
    } else if (event.key === 'ArrowDown') {
      event.preventDefault();
      move(view === 'grid' ? columns : 1, event);
    } else if (event.key === 'ArrowUp') {
      event.preventDefault();
      move(view === 'grid' ? -columns : -1, event);
    } else if (event.key === 'Enter') {
      event.preventDefault();
      if (selected) openFile(selected.id);
    } else if (event.key === 'Delete') {
      event.preventDefault();
      void askDelete([...selectedIds]);
    } else if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'c') {
      event.preventDefault();
      if (selected) copyImage(selected.id);
    }
  }
</script>

<svelte:window
  {onkeydown}
  onclick={deselectBackground}
  onpointermove={moveDrag}
  onpointerup={finishDrag}
  onpointercancel={cancelDrag}
  onblur={stopDrag}
/>

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

    {#if deleteError || errorMessage}
      <p class="error" role="status">{deleteError || errorMessage}</p>
    {/if}

    {#if selectedIds.size > 1}
      <div class="selection-bar">
        <span role="status">{selectedIds.size} selected</span>
        <button
          type="button"
          class="upload-file hv delete-selected"
          disabled={deleting}
          onclick={() => void askDelete([...selectedIds])}
        >
          <Icon name="trash" size={14} />
          Delete selected
        </button>
        <button type="button" class="upload-file hv" onclick={clearSelection}
          >Clear selection</button
        >
      </div>
    {/if}

    <div class="scroll-frame">
      <div
        class="scroll"
        role="region"
        aria-label="Capture results"
        bind:this={scroller}
        bind:clientWidth={viewportWidth}
        bind:clientHeight={viewportHeight}
        {onscroll}
        onpointerdown={beginDrag}
        onlostpointercapture={cancelDrag}
        ondragstart={(event) => event.preventDefault()}
        onclickcapture={suppressDragClick}
      >
        {#if loaded && !loading && !errorMessage && items.length === 0}
          <div class="empty">
            {#if searching || filter !== 'all'}
              <EmptyState
                title="No matching items"
                text="Clear the search or change the filter to see more of your history."
              />
              <div class="empty-actions">
                {#if searching}
                  <button type="button" class="upload-file hv" onclick={() => searchQuery.set('')}
                    >Clear search</button
                  >
                {/if}
                {#if filter !== 'all'}
                  <button type="button" class="upload-file hv" onclick={() => (filterLabel = 'All')}
                    >Reset filters</button
                  >
                {/if}
              </div>
            {:else}
              <EmptyState title="No captures yet" text="Press PrtSc to capture a region." />
              <button
                type="button"
                class="upload-file primary hv"
                onclick={() => void uploadFiles()}>Upload file…</button
              >
            {/if}
          </div>
        {:else if view === 'grid'}
          <CaptureGrid
            {groups}
            {activeUploads}
            selectedIds={highlighted}
            {focusId}
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
            selectedIds={highlighted}
            {focusId}
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
      {#if marquee}
        <div
          class="marquee"
          aria-hidden="true"
          style:left="{marquee.left}px"
          style:top="{marquee.top}px"
          style:width="{marquee.width}px"
          style:height="{marquee.height}px"
        ></div>
      {/if}
    </div>
  </main>

  {#if selected}
    <div class="detail-divider">
      <ResizeDivider
        label="Resize capture details"
        value={detailWidth}
        min={280}
        max={480}
        direction={-1}
        onresize={(width) => (detailWidth = width)}
      />
    </div>
    <DetailPane
      width={detailWidth}
      capture={selected}
      activeUpload={selected ? (activeUploads[selected.id] ?? null) : null}
      {defaultDestination}
      oncopy={copyImage}
      onshow={showInFolder}
      onopen={openFile}
      onannotate={annotate}
      onpin={pin}
      onclose={closeDetails}
      ondelete={(id) => {
        void askDelete([id]);
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

  .detail-divider {
    display: flex;
  }

  @media (max-width: 1050px) {
    .detail-divider {
      display: none;
    }
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

  .selection-bar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
    padding: 6px 8px;
    border: 1px solid var(--stroke);
    border-radius: 6px;
    background: var(--card);
    font-size: 12px;
  }

  .selection-bar span {
    flex: 1 0 auto;
  }

  .delete-selected {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    color: var(--err);
  }

  .delete-selected:disabled {
    opacity: 0.55;
    cursor: default;
  }

  .scroll-frame {
    display: flex;
    flex: 1;
    min-height: 0;
    min-width: 0;
    position: relative;
  }

  .scroll {
    flex: 1;
    min-height: 0;
    min-width: 0;
    overflow-y: auto;
    padding-bottom: 32px;
    user-select: none;
  }

  .marquee {
    position: absolute;
    z-index: 2;
    box-sizing: border-box;
    pointer-events: none;
    border: 1px solid var(--accent);
    background: var(--accentSoft);
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
