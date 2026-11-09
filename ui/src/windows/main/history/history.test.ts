import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { tick } from 'svelte';
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
vi.mock('../../../lib/ipc/upload', () => ({
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
    dest: 'Imgur',
    runId: null,
    status: 'queued' as const,
    attempts: 0,
    link: null,
    pageLink: null,
    deletionLink: null,
    error: null,
    retryInMs: null,
    ...overrides,
  };
}

async function flush(): Promise<void> {
  await tick();
  await tick();
  await Promise.resolve();
  await tick();
}

function page(items: HistoryCaptureDto[], next: { createdAt: number; id: number } | null = null) {
  return { items, next };
}

beforeEach(() => {
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
    expect(container.querySelectorAll('[data-capture-id]').length).toBeLessThan(120);
  });

  it('selects a capture and shows its details', async () => {
    const { container } = render(HistoryPage);
    await flush();
    const cards = container.querySelectorAll('[data-capture-id]');
    await fireEvent.click(cards[1] as Element);
    await flush();
    expect(container.querySelector('aside')?.textContent).toContain('capture-2.png');
  });

  it('moves the selection with the arrow keys', async () => {
    const { container } = render(HistoryPage);
    await flush();
    await fireEvent.keyDown(window, { key: 'ArrowRight' });
    await flush();
    expect(container.querySelector('aside')?.textContent).toContain('capture-2.png');
  });

  it('opens the selected image in the native editor', async () => {
    render(HistoryPage);
    await flush();
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
    expect(screen.getByRole('columnheader', { name: 'Captured' })).not.toBeNull();
    expect(container.querySelectorAll('[data-capture-id]').length).toBe(3);
  });

  it('shows the empty state when nothing matches', async () => {
    mocks.page.mockResolvedValue(page([]));
    render(HistoryPage);
    await flush();
    expect(screen.getByText('No captures yet')).not.toBeNull();
    expect(screen.getByText('Press PrtSc to capture a region.')).not.toBeNull();
  });

  it('opens the selected file with Enter and copies it with Ctrl+C', async () => {
    render(HistoryPage);
    await flush();
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
    expect(screen.getByText('Delete from Amazon S3')).not.toBeNull();
  });

  it('uploads a local capture to the default destination and can cancel it', async () => {
    render(HistoryPage);
    await flush();
    expect(
      screen.getByText('Saved on this PC only. Upload it to get a shareable link.'),
    ).not.toBeNull();
    await fireEvent.click(screen.getByRole('button', { name: 'Upload to Imgur' }));
    await flush();
    expect(uploads.call).toHaveBeenCalledWith('ipc_upload', {
      kind: 'uploadElsewhere',
      data: { capture_id: 1, dest: 'Imgur' },
    });
    expect(screen.getByRole('progressbar', { name: 'Uploading' })).not.toBeNull();
    await fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    await flush();
    expect(uploads.call).toHaveBeenLastCalledWith('ipc_upload', {
      kind: 'cancel',
      data: { id: 42 },
    });
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
    mocks.page.mockResolvedValue(
      page([
        capture(1, {
          upload: link({ state: 'failed', url: null, error: 'The upload failed.' }),
        }),
      ]),
    );
    render(HistoryPage);
    await flush();
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
    let handler: ((event: AppEvent) => void) | undefined;
    uploads.onEvent.mockImplementation((callback: (event: AppEvent) => void) => {
      handler = callback;
      return Promise.resolve(() => undefined);
    });
    render(HistoryPage);
    await flush();
    await fireEvent.click(screen.getByRole('button', { name: 'Upload to Imgur' }));
    await flush();
    handler?.({
      area: 'upload',
      event: {
        kind: 'stateChanged',
        data: uploadState(42, {
          status: 'failed',
          error: 'The host refused the file.',
        }),
      },
    });
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
