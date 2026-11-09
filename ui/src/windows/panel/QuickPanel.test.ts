import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { tick } from 'svelte';
import QuickPanel from './QuickPanel.svelte';
import { panelRecents } from './data';

const ipc = vi.hoisted(() => ({
  panelResize: vi.fn(() => Promise.resolve({ kind: 'panelResize' })),
  panelClose: vi.fn(() => Promise.resolve({ kind: 'panelClose' })),
  panelAction: vi.fn(() => Promise.resolve({ kind: 'panelAction' })),
  panelCopyLink: vi.fn(() => Promise.resolve({ kind: 'panelCopyLink' })),
  openMain: vi.fn(() => Promise.resolve({ kind: 'openMain' })),
  openCapture: vi.fn(() => Promise.resolve({ kind: 'openCapture' })),
}));

vi.mock('./ipc', () => ipc);

beforeEach(() => {
  for (const mock of Object.values(ipc)) {
    mock.mockClear();
  }
});

afterEach(cleanup);

describe('QuickPanel', () => {
  it('renders the header, capture tiles, recents and footer', async () => {
    render(QuickPanel);
    await tick();
    expect(screen.getByRole('main', { name: 'Snapline quick panel' })).not.toBeNull();
    expect(screen.getByText('Snapline')).not.toBeNull();
    expect(screen.getByRole('button', { name: 'Open Snapline' })).not.toBeNull();
    expect(screen.getByRole('button', { name: 'Settings' })).not.toBeNull();
    expect(screen.getByText('Recent')).not.toBeNull();
    expect(screen.getByText('View all')).not.toBeNull();
    expect(screen.getByText('Last link')).not.toBeNull();
    expect(screen.getByText('i.imgur.com/kq7Ra2P.png')).not.toBeNull();
    expect(screen.getByRole('button', { name: 'Copy' })).not.toBeNull();
    for (const recent of panelRecents) {
      expect(screen.getByRole('button', { name: recent.name })).not.toBeNull();
    }
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

  it('disables Copy when there is no last link', () => {
    render(QuickPanel, { props: { lastLink: null } });
    const copy = screen.getByRole('button', { name: 'Copy' }) as HTMLButtonElement;
    expect(copy.disabled).toBe(true);
    expect(screen.getByText('None yet')).not.toBeNull();
  });

  it('shows the empty state when there are no recents', () => {
    render(QuickPanel, { props: { recents: [] } });
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
    await fireEvent.click(
      screen.getByRole('button', { name: panelRecents[0]?.name ?? '' }),
    );
    expect(ipc.openCapture).toHaveBeenCalledWith(1);
    await fireEvent.click(screen.getByText('View all'));
    expect(ipc.openMain).toHaveBeenCalledWith('/history');
  });

  it('copies the last link when it is present', async () => {
    render(QuickPanel);
    await fireEvent.click(screen.getByRole('button', { name: 'Copy' }));
    expect(ipc.panelCopyLink).toHaveBeenCalledOnce();
  });
});
