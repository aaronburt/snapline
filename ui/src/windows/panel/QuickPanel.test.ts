import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { tick } from 'svelte';
import type { RecentCaptureDto } from '../../lib/ipc/bindings/RecentCaptureDto';
import QuickPanel from './QuickPanel.svelte';
import { relativeTime } from './data';

const core = vi.hoisted(() => ({
  call: vi.fn(),
}));

vi.mock('../../lib/ipc/core', () => core);

const ipc = vi.hoisted(() => ({
  panelResize: vi.fn(() => Promise.resolve({ kind: 'panelResize' })),
  panelClose: vi.fn(() => Promise.resolve({ kind: 'panelClose' })),
  panelAction: vi.fn(() => Promise.resolve({ kind: 'panelAction' })),
  panelCopyLink: vi.fn(() => Promise.resolve({ kind: 'panelCopyLink' })),
  openMain: vi.fn(() => Promise.resolve({ kind: 'openMain' })),
  openCapture: vi.fn(() => Promise.resolve({ kind: 'openCapture' })),
  panelRecent: vi.fn(),
}));

vi.mock('./ipc', () => ipc);

function recent(id: number, overrides: Partial<RecentCaptureDto> = {}): RecentCaptureDto {
  return {
    id,
    name: `capture-${id}.png`,
    kind: 'image',
    status: 'local',
    createdAt: Date.now() - id * 60_000,
    url: null,
    durationMs: null,
    ...overrides,
  };
}

async function flush(): Promise<void> {
  await tick();
  await tick();
  await Promise.resolve();
  await tick();
}

beforeEach(() => {
  for (const mock of Object.values(ipc)) {
    mock.mockClear();
  }
  core.call.mockReset();
  core.call.mockResolvedValue({
    kind: 'lastLink',
    data: { link: 'https://i.imgur.com/kq7Ra2P.png' },
  });
  ipc.panelRecent.mockResolvedValue([recent(1), recent(2), recent(3)]);
});

afterEach(cleanup);

describe('QuickPanel', () => {
  it('renders the header, capture tiles, real recents and footer', async () => {
    render(QuickPanel);
    await flush();
    expect(screen.getByRole('main', { name: 'Snapline quick panel' })).not.toBeNull();
    expect(screen.getByText('Snapline')).not.toBeNull();
    expect(screen.getByRole('button', { name: 'Open Snapline' })).not.toBeNull();
    expect(screen.getByRole('button', { name: 'Settings' })).not.toBeNull();
    expect(screen.getByText('Recent')).not.toBeNull();
    expect(screen.getByText('View all')).not.toBeNull();
    expect(screen.getByText('Last link')).not.toBeNull();
    expect(screen.getByText('i.imgur.com/kq7Ra2P.png')).not.toBeNull();
    expect(screen.getByRole('button', { name: 'Copy' })).not.toBeNull();
    expect(ipc.panelRecent).toHaveBeenCalledOnce();
    expect(ipc.panelResize).toHaveBeenCalled();
  });

  it('keeps the tile order and shows chord tooltips', () => {
    const { container } = render(QuickPanel);
    const tiles = Array.from(container.querySelectorAll('button[title]'));
    expect(tiles.map((tile) => tile.getAttribute('title'))).toEqual([
      'Region (PrtSc)',
      'Window (Alt+PrtSc)',
      'Screen (Ctrl+PrtSc)',
      'Scrolling (Ctrl+Alt+S)',
    ]);
    expect(tiles.map((tile) => tile.textContent?.trim())).toEqual([
      'Region',
      'Window',
      'Screen',
      'Scrolling',
    ]);
  });

  it('renders the three newest captures in the order the service returned them', async () => {
    ipc.panelRecent.mockResolvedValue([recent(9), recent(8), recent(7)]);
    const { container } = render(QuickPanel);
    await flush();
    const images = Array.from(container.querySelectorAll('.recent-item img'));
    expect(images.map((image) => image.getAttribute('src'))).toEqual([
      'http://thumb.localhost/9',
      'http://thumb.localhost/8',
      'http://thumb.localhost/7',
    ]);
    expect(screen.getByRole('button', { name: 'capture-9.png' })).not.toBeNull();
    expect(screen.getByRole('button', { name: 'capture-8.png' })).not.toBeNull();
    expect(screen.getByRole('button', { name: 'capture-7.png' })).not.toBeNull();
  });

  it('shows a duration badge for videos and GIFs', async () => {
    ipc.panelRecent.mockResolvedValue([
      recent(4, { kind: 'video', name: 'clip.mp4', durationMs: 42_000 }),
      recent(5, { kind: 'gif', name: 'loop.gif', durationMs: 8_000 }),
      recent(6),
    ]);
    render(QuickPanel);
    await flush();
    expect(screen.getByText('0:42')).not.toBeNull();
    expect(screen.getByText('0:08')).not.toBeNull();
  });

  it('shows the green link icon only for uploaded captures', async () => {
    ipc.panelRecent.mockResolvedValue([
      recent(1, {
        status: 'uploaded',
        url: 'https://example.com/capture-1.png',
      }),
      recent(2),
      recent(3, { status: 'failed' }),
    ]);
    render(QuickPanel);
    await flush();
    expect(screen.getAllByRole('img', { name: 'Uploaded' })).toHaveLength(1);
  });

  it('disables Copy when there is no last link', async () => {
    core.call.mockResolvedValue({ kind: 'lastLink', data: { link: null } });
    render(QuickPanel);
    await flush();
    const copy = screen.getByRole('button', { name: 'Copy' }) as HTMLButtonElement;
    expect(copy.disabled).toBe(true);
    expect(screen.getByText('None yet')).not.toBeNull();
  });

  it('reads the persisted last link on mount and shows it without the scheme', async () => {
    core.call.mockResolvedValue({
      kind: 'lastLink',
      data: { link: 'https://files.example.com/very/long/link.png' },
    });
    render(QuickPanel);
    await flush();
    expect(core.call).toHaveBeenCalledWith('ipc_upload', { kind: 'lastLink' });
    expect(screen.getByText('files.example.com/very/long/link.png')).not.toBeNull();
    const copy = screen.getByRole('button', { name: 'Copy' }) as HTMLButtonElement;
    expect(copy.disabled).toBe(false);
  });

  it('shows the empty state when there are no captures', async () => {
    ipc.panelRecent.mockResolvedValue([]);
    render(QuickPanel);
    await flush();
    expect(screen.getByText('No captures yet')).not.toBeNull();
  });

  it('closes the panel on Escape', async () => {
    render(QuickPanel);
    await fireEvent.keyDown(document.body, { key: 'Escape' });
    expect(ipc.panelClose).toHaveBeenCalledOnce();
  });

  it('runs the Region action and opens the main window from the header', async () => {
    render(QuickPanel);
    await fireEvent.click(screen.getByTitle('Region (PrtSc)'));
    expect(ipc.panelAction).toHaveBeenCalledWith('capture_region');
    await fireEvent.click(screen.getByRole('button', { name: 'Open Snapline' }));
    expect(ipc.openMain).toHaveBeenCalledWith('/history');
    await fireEvent.click(screen.getByRole('button', { name: 'Settings' }));
    expect(ipc.openMain).toHaveBeenCalledWith('/settings/general');
  });

  it('opens a recent capture and the full History list', async () => {
    render(QuickPanel);
    await flush();
    await fireEvent.click(screen.getByRole('button', { name: 'capture-1.png' }));
    expect(ipc.openCapture).toHaveBeenCalledWith(1);
    await fireEvent.click(screen.getByText('View all'));
    expect(ipc.openMain).toHaveBeenCalledWith('/history');
  });

  it('copies the last link when it is present', async () => {
    render(QuickPanel);
    await flush();
    await fireEvent.click(screen.getByRole('button', { name: 'Copy' }));
    expect(ipc.panelCopyLink).toHaveBeenCalledOnce();
  });
});

describe('relativeTime', () => {
  const now = new Date(2026, 10, 9, 15, 30, 0).getTime();
  const minute = 60_000;
  const hour = 60 * minute;

  it('uses minutes under an hour', () => {
    expect(relativeTime(now - 2 * minute, now)).toBe('2 min ago');
    expect(relativeTime(now - 18 * minute, now)).toBe('18 min ago');
    expect(relativeTime(now - 59 * minute, now)).toBe('59 min ago');
  });

  it('uses hours up to the end of today', () => {
    expect(relativeTime(now - hour, now)).toBe('1 h ago');
    expect(relativeTime(now - 5 * hour, now)).toBe('5 h ago');
  });

  it('uses Yesterday for the previous calendar day', () => {
    expect(relativeTime(new Date(2026, 10, 8, 23, 0, 0).getTime(), now)).toBe('Yesterday');
    expect(relativeTime(new Date(2026, 10, 8, 9, 0, 0).getTime(), now)).toBe('Yesterday');
  });

  it('uses the day and month for older captures', () => {
    expect(relativeTime(new Date(2026, 10, 7, 12, 0, 0).getTime(), now)).toBe('7 Nov');
    expect(relativeTime(new Date(2026, 10, 8, 23, 59, 0).getTime(), now)).toBe('Yesterday');
  });

  it('never shows zero minutes', () => {
    expect(relativeTime(now - 5_000, now)).toBe('1 min ago');
  });
});
