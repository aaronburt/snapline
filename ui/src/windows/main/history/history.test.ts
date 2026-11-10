import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { flush, MIDDAY } from '../../../test/flush';
import { expectAccessible } from '../../../test/axe';
import type { AppEvent } from '../../../lib/ipc/bindings/AppEvent';
import type { HistoryCaptureDto } from '../../../lib/ipc/bindings/HistoryCaptureDto';
import type { UploadState } from '../../../lib/ipc/bindings/UploadState';
import HistoryPage from './HistoryPage.svelte';
import { formatBytes, formatDuration, groupCaptures } from './data';
import { searchQuery } from './search';

const uploads = vi.hoisted(() => ({
  call: vi.fn(),
  destinations: vi.fn(),
  copyLink: vi.fn(),
  onEvent: vi.fn(),
}));

vi.mock('../../../lib/ipc/core', () => ({ call: uploads.call }));
vi.mock('../../../lib/ipc/upload', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../../../lib/ipc/upload')>()),
  listDestinations: uploads.destinations,
  copyLink: uploads.copyLink,
}));
vi.mock('../../../lib/events', () => ({ onEvent: uploads.onEvent }));

const mocks = vi.hoisted(() => ({
  page: vi.fn(),
  search: vi.fn(),
  stats: vi.fn(),
  confirm: vi.fn(),
  remove: vi.fn(),
  copy: vi.fn(),
  show: vi.fn(),
  open: vi.fn(),
  openUrl: vi.fn(),
  settings: vi.fn(),
  editor: vi.fn(),
}));

vi.mock('../../../lib/ipc/history', () => ({
  historyPage: mocks.page,
  historySearch: mocks.search,
  historyStats: mocks.stats,
  historyConfirmDelete: mocks.confirm,
  historyDelete: mocks.remove,
  historyCopyImage: mocks.copy,
  historyShowInFolder: mocks.show,
  historyOpenFile: mocks.open,
  historyOpenUrl: mocks.openUrl,
}));

vi.mock('../../../lib/ipc/settings', () => ({ get: mocks.settings }));
vi.mock('../../../lib/ipc/tool', () => ({ editorOpenHistory: mocks.editor }));

function capture(id: number, overrides: Partial<HistoryCaptureDto> = {}): HistoryCaptureDto {
  return {
    id,
    kind: 'image',
    owned: true,
    missing: false,
    name: `capture-${id}.png`,
    bytes: 412 * 1024,
    width: 1280,
    height: 720,
    durationMs: null,
    sourceApp: 'chrome.exe',
    createdAt: Date.now() - id * 60_000,
    folder: 'Pictures\\Snapline\\2026-11',
    upload: null,
    ...overrides,
  };
}

function link(overrides: Partial<NonNullable<HistoryCaptureDto['upload']>> = {}) {
  return {
    destId: 'imgur',
    state: 'done' as const,
    sent: 0,
    total: 0,
    url: 'https://i.imgur.com/abc123.png',
    deletionUrl: null,
    error: null,
    attempts: 1,
    updatedAt: 1_760_000_000_000,
    ...overrides,
  };
}

function uploadState(id: number, overrides: Partial<UploadState> = {}): UploadState {
  return {
    id,
    captureId: null,
    fileName: `upload-${id}.png`,
    dest: 'Imgur',
    runId: null,
    status: 'queued' as const,
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
    ...overrides,
  };
}

async function selectFirst(): Promise<void> {
  const card = document.querySelector('[data-capture-id="1"]');
  if (!card) throw new Error('Capture not found');
  await fireEvent.click(card);
  await flush();
}

function page(items: HistoryCaptureDto[], next: { createdAt: number; id: number } | null = null) {
  return { items, next };
}

beforeEach(() => {
  vi.useFakeTimers({ toFake: ['Date'] });
  vi.setSystemTime(MIDDAY);
  searchQuery.set('');
  uploads.call.mockReset();
  uploads.destinations.mockReset();
  uploads.onEvent.mockReset();
  uploads.call.mockResolvedValue({ kind: 'accepted', data: { id: 42 } });
  uploads.destinations.mockResolvedValue([
    { id: 's3-prod', name: 'Amazon S3', configured: true },
    { id: 'unfinished', name: 'Unfinished', configured: false },
  ]);
  uploads.onEvent.mockResolvedValue(() => undefined);
  mocks.page.mockReset();
  mocks.search.mockReset();
  mocks.stats.mockReset();
  mocks.confirm.mockReset();
  mocks.remove.mockReset();
  mocks.copy.mockReset();
  mocks.show.mockReset();
  mocks.open.mockReset();
  mocks.openUrl.mockReset();
  mocks.settings.mockReset();
  mocks.editor.mockReset();
  mocks.page.mockResolvedValue(page([capture(1), capture(2), capture(3)]));
  mocks.search.mockResolvedValue(page([capture(7)]));
  mocks.stats.mockResolvedValue({ count: 3, bytes: 3 * 412 * 1024, uploadedThisWeek: 1 });
  mocks.confirm.mockResolvedValue(true);
  mocks.remove.mockResolvedValue(undefined);
  mocks.copy.mockResolvedValue(undefined);
  mocks.show.mockResolvedValue(undefined);
  mocks.open.mockResolvedValue(undefined);
  mocks.openUrl.mockResolvedValue(undefined);
  mocks.settings.mockResolvedValue({ uploads: { images: 'Imgur' } });
  mocks.editor.mockResolvedValue({
    action: null,
    captureId: null,
    path: null,
    cancelled: true,
  });
});

afterEach(() => {
  cleanup();
  searchQuery.set('');
  vi.useRealTimers();
});

describe('History data helpers', () => {
  function at(daysAgo: number): number {
    const date = new Date();
    date.setDate(date.getDate() - daysAgo);
    date.setHours(12, 0, 0, 0);
    return date.getTime();
  }

  it('groups captures by day with counts', () => {
    const groups = groupCaptures([
      capture(1, { createdAt: at(0) }),
      capture(2, { createdAt: at(0) + 60_000 }),
      capture(3, { createdAt: at(1) }),
      capture(4, { createdAt: at(2) }),
    ]);
    expect(groups.map((group) => group.label).slice(0, 2)).toEqual(['Today', 'Yesterday']);
    expect(groups).toHaveLength(3);
    expect(groups[0].countLabel).toBe('2 captures');
    expect(groups[1].countLabel).toBe('1 capture');
    expect(['Today', 'Yesterday']).not.toContain(groups[2].label);
  });

  it('formats sizes, durations and types in UK style', () => {
    expect(formatBytes(412 * 1024)).toBe('412 KB');
    expect(formatBytes(18.4 * 1024 * 1024)).toBe('18.4 MB');
    expect(formatDuration(42_000)).toBe('0:42');
    expect(formatDuration(118_000)).toBe('1:58');
  });
});

describe('History page', () => {
  it('renders real captures and the stats subtitle', async () => {
    const { container } = render(HistoryPage);
    await flush();
    await expectAccessible(container);
    expect(screen.getByRole('heading', { level: 1, name: 'History' })).not.toBeNull();
    expect(screen.getByText(/3 captures · 1.2 MB on disk · 1 uploaded this week/)).not.toBeNull();
    expect(screen.getByRole('heading', { level: 2, name: /Today/ })).not.toBeNull();
    expect(container.querySelectorAll('[data-capture-id]').length).toBe(3);
  });

  it('asks the service for the chosen filter', async () => {
    render(HistoryPage);
    await flush();
    await fireEvent.click(screen.getByRole('radio', { name: 'Videos' }));
    await flush();
    expect(mocks.page).toHaveBeenLastCalledWith('videos', null);
  });

  it('searches through the title bar query', async () => {
    render(HistoryPage);
    await flush();
    searchQuery.set('capture-7');
    await flush();
    expect(mocks.search).toHaveBeenLastCalledWith('capture-7', 'all', null);
  });

  it('loads the next page when the list is scrolled near the bottom', async () => {
    const first = Array.from({ length: 60 }, (_, index) => capture(index + 1));
    const second = Array.from({ length: 60 }, (_, index) => capture(index + 61));
    const cursor = { createdAt: 1, id: 60 };
    mocks.page.mockResolvedValueOnce(page(first, cursor));
    mocks.page.mockResolvedValueOnce(page(second));
    const { container } = render(HistoryPage);
    await flush();
    expect(mocks.page).toHaveBeenCalledTimes(1);
    const scroll = container.querySelector('.scroll') as HTMLElement;
    expect(scroll).not.toBeNull();
    scroll.scrollTop = 900;
    await fireEvent.scroll(scroll);
    await flush();
    expect(mocks.page).toHaveBeenCalledTimes(2);
    expect(mocks.page).toHaveBeenLastCalledWith('all', cursor);
    const rendered = container.querySelectorAll('[data-capture-id]').length;
    expect(rendered).toBeGreaterThan(0);
    expect(rendered).toBeLessThan(120);
  });

  it('selects a capture and shows its details', async () => {
    const { container } = render(HistoryPage);
    await flush();
    expect(screen.queryByRole('complementary', { name: 'Capture details' })).toBeNull();
    const cards = container.querySelectorAll('[data-capture-id]');
    await fireEvent.click(cards[1] as Element);
    await flush();
    expect(container.querySelector('aside')?.textContent).toContain('capture-2.png');
  });

  it('keeps capture cards a fixed width when details open and close', async () => {
    const { container } = render(HistoryPage);
    await flush();
    const row = container.querySelector('.gridrow') as HTMLElement;
    expect(row.style.gridTemplateColumns).toContain('200px');
    await selectFirst();
    expect(
      (container.querySelector('.gridrow') as HTMLElement).style.gridTemplateColumns,
    ).toContain('200px');
    await fireEvent.click(container.querySelector('.scroll') as Element);
    await flush();
    expect(
      (container.querySelector('.gridrow') as HTMLElement).style.gridTemplateColumns,
    ).toContain('200px');
  });

  it('deselects a capture when the history background is clicked', async () => {
    const { container } = render(HistoryPage);
    await flush();
    await selectFirst();
    expect(screen.getByRole('complementary', { name: 'Capture details' })).not.toBeNull();
    await fireEvent.click(container.querySelector('.scroll') as Element);
    await flush();
    expect(screen.queryByRole('complementary', { name: 'Capture details' })).toBeNull();
  });

  it('moves the selection with the arrow keys', async () => {
    const { container } = render(HistoryPage);
    await flush();
    await selectFirst();
    await fireEvent.keyDown(window, { key: 'ArrowRight' });
    await flush();
    expect(container.querySelector('aside')?.textContent).toContain('capture-2.png');
  });

  it('opens the selected image in the native editor', async () => {
    render(HistoryPage);
    await flush();
    await selectFirst();
    await fireEvent.click(screen.getByRole('button', { name: 'Annotate' }));
    await waitFor(() => expect(mocks.editor).toHaveBeenCalledWith(1));
  });

  it('deletes only after the native confirmation', async () => {
    const { container } = render(HistoryPage);
    await flush();
    const cards = container.querySelectorAll('[data-capture-id]');
    await fireEvent.click(cards[1] as Element);
    await flush();
    await fireEvent.click(screen.getByRole('button', { name: 'Delete' }));
    await waitFor(() => expect(mocks.remove).toHaveBeenCalledWith(2));
    expect(mocks.confirm).toHaveBeenCalledWith(2);
    expect(container.querySelector('[data-capture-id="2"]')).toBeNull();
  });

  it('keeps the capture when the confirmation is cancelled', async () => {
    mocks.confirm.mockResolvedValue(false);
    const { container } = render(HistoryPage);
    await flush();
    await selectFirst();
    await fireEvent.click(screen.getByRole('button', { name: 'Delete' }));
    await flush();
    expect(mocks.remove).not.toHaveBeenCalled();
    expect(container.querySelector('[data-capture-id="1"]')).not.toBeNull();
  });

  it('switches to the virtual list view', async () => {
    const { container } = render(HistoryPage);
    await flush();
    await fireEvent.click(screen.getByRole('button', { name: 'List view' }));
    await flush();
    expect(container.querySelector('table')).not.toBeNull();
    expect(screen.getByRole('columnheader', { name: 'Status' })).not.toBeNull();
    expect(container.querySelectorAll('[data-capture-id]').length).toBe(3);
  });

  it('offers an upload action when History is empty', async () => {
    mocks.page.mockResolvedValue(page([]));
    uploads.call.mockResolvedValue({ kind: 'acceptedMany', data: { ids: [] } });
    render(HistoryPage);
    await flush();
    expect(screen.getByText('No captures yet')).not.toBeNull();
    expect(screen.getByText('Press PrtSc to capture a region.')).not.toBeNull();
    const actions = screen.getAllByRole('button', { name: 'Upload file…' });
    expect(actions).toHaveLength(2);
    await fireEvent.click(actions[1]);
    expect(uploads.call).toHaveBeenCalledWith('ipc_upload', { kind: 'pickAndUpload' });
  });

  it('lets an empty search return to the current filter', async () => {
    mocks.search.mockResolvedValue(page([]));
    searchQuery.set('no such file');
    render(HistoryPage);
    await flush();
    expect(screen.getByText('No matching items')).not.toBeNull();
    await fireEvent.click(screen.getByRole('button', { name: 'Clear search' }));
    await flush();
    expect(screen.queryByText('No matching items')).toBeNull();
    expect(mocks.page).toHaveBeenCalledWith('all', null);
  });

  it('closes Details and restores focus to the selected item', async () => {
    const { container } = render(HistoryPage);
    await flush();
    await selectFirst();
    await fireEvent.click(screen.getByRole('button', { name: 'Close details' }));
    await flush();
    expect(screen.queryByRole('complementary', { name: 'Capture details' })).toBeNull();
    expect(document.activeElement).toBe(container.querySelector('[data-capture-id="1"]'));
  });

  it('opens the selected file with Enter and copies it with Ctrl+C', async () => {
    render(HistoryPage);
    await flush();
    await selectFirst();
    await fireEvent.keyDown(window, { key: 'Enter' });
    await flush();
    expect(mocks.open).toHaveBeenCalledWith(1);
    await fireEvent.keyDown(window, { key: 'c', ctrlKey: true });
    await flush();
    expect(mocks.copy).toHaveBeenCalledWith(1);
  });
});

describe('History sharing', () => {
  beforeEach(() => {
    uploads.copyLink.mockReset();
    uploads.copyLink.mockResolvedValue(undefined);
  });

  it('shows the uploaded link, copies it and offers the deletion link without a QR button', async () => {
    mocks.page.mockResolvedValue(
      page([
        capture(1, {
          upload: link({ deletionUrl: 'https://imgur.com/delete/abc123' }),
        }),
      ]),
    );
    render(HistoryPage);
    await flush();
    await selectFirst();
    const input = screen.getByLabelText('Share link') as HTMLInputElement;
    expect(input.value).toBe('https://i.imgur.com/abc123.png');
    expect(input.readOnly).toBe(true);
    expect(screen.queryByRole('button', { name: 'Show QR code' })).toBeNull();
    await fireEvent.click(screen.getByRole('button', { name: 'Copy link' }));
    await flush();
    expect(uploads.copyLink).toHaveBeenCalledWith('https://i.imgur.com/abc123.png');
    expect(screen.getByText('Deletion link saved')).not.toBeNull();
    await fireEvent.click(screen.getByRole('button', { name: 'Copy deletion link' }));
    await flush();
    expect(uploads.copyLink).toHaveBeenCalledWith('https://imgur.com/delete/abc123');
  });

  it('offers Delete from the host when the host supports API deletion', async () => {
    mocks.page.mockResolvedValue(
      page([capture(1, { upload: link({ destId: 's3', url: 'https://cdn.example.com/a.png' }) })]),
    );
    render(HistoryPage);
    await flush();
    await selectFirst();
    expect(screen.getByText('Delete from Amazon S3')).not.toBeNull();
  });

  it('hides sharing for a local capture', async () => {
    render(HistoryPage);
    await flush();
    await selectFirst();
    expect(screen.queryByRole('region', { name: 'Sharing' })).toBeNull();
  });

  it('retries a failed upload in place', async () => {
    mocks.page.mockResolvedValue(
      page([
        capture(1, {
          upload: link({
            state: 'failed',
            url: null,
            error: 'The server returned HTTP 500.',
          }),
        }),
      ]),
    );
    render(HistoryPage);
    await flush();
    await selectFirst();
    expect(screen.getByText('Upload failed: The server returned HTTP 500.')).not.toBeNull();
    await fireEvent.click(screen.getByRole('button', { name: 'Retry' }));
    await flush();
    expect(uploads.call).toHaveBeenCalledWith('ipc_upload', {
      kind: 'uploadElsewhere',
      data: { capture_id: 1, dest: 'imgur' },
    });
  });

  it('lists the configured destinations in Upload elsewhere and sends only the destination id', async () => {
    mocks.page.mockResolvedValue(
      page([
        capture(1, {
          upload: link({ state: 'failed', url: null, error: 'The upload failed.' }),
        }),
      ]),
    );
    render(HistoryPage);
    await flush();
    await selectFirst();
    await fireEvent.click(screen.getByRole('button', { name: 'Upload elsewhere…' }));
    await flush();
    expect(uploads.destinations).toHaveBeenCalledOnce();
    expect(screen.queryByRole('menuitem', { name: 'Unfinished' })).toBeNull();
    await fireEvent.click(screen.getByRole('menuitem', { name: 'Amazon S3' }));
    await flush();
    expect(uploads.call).toHaveBeenCalledWith('ipc_upload', {
      kind: 'uploadElsewhere',
      data: { capture_id: 1, dest: 's3-prod' },
    });
  });

  it('operates the Upload elsewhere menu with the keyboard and returns focus', async () => {
    uploads.destinations.mockResolvedValue([
      { id: 's3-prod', name: 'Amazon S3', configured: true },
      { id: 'imgur', name: 'Imgur', configured: true },
    ]);
    mocks.page.mockResolvedValue(
      page([
        capture(1, {
          upload: link({ state: 'failed', url: null, error: 'The upload failed.' }),
        }),
      ]),
    );
    render(HistoryPage);
    await flush();
    await selectFirst();
    const trigger = screen.getByRole('button', { name: 'Upload elsewhere…' });
    await fireEvent.click(trigger);
    await flush();
    const menu = screen.getByRole('menu', { name: 'Upload elsewhere' });
    const items = screen.getAllByRole('menuitem');
    expect(document.activeElement).toBe(items[0]);
    await fireEvent.keyDown(menu, { key: 'ArrowDown' });
    expect(document.activeElement).toBe(items[1]);
    await fireEvent.keyDown(menu, { key: 'Escape' });
    await flush();
    expect(screen.queryByRole('menu')).toBeNull();
    expect(document.activeElement).toBe(trigger);
  });

  it('retries through the queue once a live upload is known', async () => {
    const handlers: ((event: AppEvent) => void)[] = [];
    uploads.onEvent.mockImplementation((callback: (event: AppEvent) => void) => {
      handlers.push(callback);
      return Promise.resolve(() => undefined);
    });
    render(HistoryPage);
    await flush();
    await selectFirst();
    handlers.forEach((handler) =>
      handler({
        area: 'upload',
        event: {
          kind: 'stateChanged',
          data: uploadState(42, {
            captureId: 1,
            status: 'failed',
            error: 'The host refused the file.',
          }),
        },
      }),
    );
    await flush();
    expect(screen.getByText('Upload failed: The host refused the file.')).not.toBeNull();
    await fireEvent.click(screen.getByRole('button', { name: 'Retry' }));
    await flush();
    expect(uploads.call).toHaveBeenLastCalledWith('ipc_upload', {
      kind: 'retry',
      data: { id: 42 },
    });
  });
});

describe('History uploads in progress', () => {
  let handlers: ((event: AppEvent) => void)[] = [];

  beforeEach(() => {
    handlers = [];
    uploads.onEvent.mockImplementation((callback: (event: AppEvent) => void) => {
      handlers.push(callback);
      return Promise.resolve(() => undefined);
    });
  });

  async function emit(state: UploadState): Promise<void> {
    handlers.forEach((handler) =>
      handler({ area: 'upload', event: { kind: 'stateChanged', data: state } }),
    );
    await flush();
  }

  function uploading(overrides: Partial<UploadState> = {}): UploadState {
    return uploadState(42, {
      captureId: 1,
      status: 'uploading',
      progressAvailable: true,
      sent: 30,
      total: 120,
      ...overrides,
    });
  }

  function bars(): HTMLProgressElement[] {
    return screen.getAllByRole('progressbar', { name: 'Upload progress' }) as HTMLProgressElement[];
  }

  it('shows the bytes sent on the card and in Details', async () => {
    const { container } = render(HistoryPage);
    await flush();
    expect(screen.queryByRole('progressbar')).toBeNull();
    await emit(uploading());
    expect(bars().map((bar) => [bar.value, bar.max])).toEqual([[30, 120]]);
    const card = container.querySelector('[data-capture-id="1"]') as HTMLElement;
    expect(card.contains(bars()[0])).toBe(true);
    await selectFirst();
    expect(bars().map((bar) => [bar.value, bar.max])).toEqual([
      [30, 120],
      [30, 120],
    ]);
    expect(container.querySelector('aside')?.textContent).toContain('30 bytes of 120 bytes');
    await emit(uploading({ sent: 90 }));
    expect(bars().map((bar) => bar.value)).toEqual([90, 90]);
  });

  it('shows an indeterminate bar when the destination cannot report progress', async () => {
    const { container } = render(HistoryPage);
    await flush();
    await emit(uploading({ progressAvailable: false, sent: 0, total: 120 }));
    await selectFirst();
    for (const bar of bars()) {
      expect(bar.hasAttribute('value')).toBe(false);
    }
    expect(container.querySelector('aside')?.textContent).toContain('Sending file…');
  });

  it('cancels from Details with the upload id', async () => {
    uploads.call.mockResolvedValue({ kind: 'cancelled', data: { id: 42 } });
    render(HistoryPage);
    await flush();
    await emit(uploading());
    await selectFirst();
    await fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    await flush();
    expect(uploads.call).toHaveBeenLastCalledWith('ipc_upload', {
      kind: 'cancel',
      data: { id: 42 },
    });
  });

  it('hides Cancel in Details when the upload cannot be cancelled', async () => {
    render(HistoryPage);
    await flush();
    await emit(uploading({ cancellable: false }));
    await selectFirst();
    expect(bars()).toHaveLength(2);
    expect(screen.queryByRole('button', { name: 'Cancel' })).toBeNull();
  });

  it('cancels from the list row with the upload id', async () => {
    uploads.call.mockResolvedValue({ kind: 'cancelled', data: { id: 42 } });
    const { container } = render(HistoryPage);
    await flush();
    await fireEvent.click(screen.getByRole('button', { name: 'List view' }));
    await flush();
    await emit(uploading());
    const row = container.querySelector('[data-capture-id="1"]') as HTMLElement;
    expect(row.contains(bars()[0])).toBe(true);
    const cancel = screen.getByRole('button', { name: 'Cancel upload of capture-1.png' });
    expect(row.contains(cancel)).toBe(true);
    await fireEvent.click(cancel);
    await flush();
    expect(uploads.call).toHaveBeenLastCalledWith('ipc_upload', {
      kind: 'cancel',
      data: { id: 42 },
    });
    expect(screen.queryByRole('complementary', { name: 'Capture details' })).toBeNull();
  });

  it('labels queued and retrying uploads and clears the bar when the upload ends', async () => {
    const { container } = render(HistoryPage);
    await flush();
    const card = (): string =>
      (container.querySelector('[data-capture-id="1"]') as HTMLElement).textContent ?? '';
    await emit(uploading({ status: 'queued', sent: 0 }));
    expect(card()).toContain('Queued');
    expect(screen.queryByRole('progressbar')).toBeNull();
    await emit(uploading());
    expect(bars()).toHaveLength(1);
    await emit(uploading({ status: 'backingOff', retryInMs: 2000 }));
    expect(card()).toContain('Waiting to retry');
    expect(screen.queryByRole('progressbar')).toBeNull();
    await emit(uploading({ status: 'cancelled', cancellable: false }));
    expect(card()).not.toContain('Waiting to retry');
    expect(screen.queryByRole('progressbar')).toBeNull();
  });
});
