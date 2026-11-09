import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import ToolsPage from './ToolsPage.svelte';

const mocks = vi.hoisted(() => ({
  available: vi.fn(),
  openFile: vi.fn(),
}));

vi.mock('../../../lib/ipc/tool', () => ({
  toolAvailable: mocks.available,
  editorOpenFile: mocks.openFile,
}));

beforeEach(() => {
  mocks.available.mockReset();
  mocks.openFile.mockReset();
  mocks.available.mockResolvedValue(true);
  mocks.openFile.mockResolvedValue({
    action: null,
    captureId: null,
    path: null,
    cancelled: true,
  });
});

afterEach(cleanup);

describe('Tools page', () => {
  it('renders the eight cards in order', async () => {
    const { container } = render(ToolsPage);
    await waitFor(() => expect(mocks.available).toHaveBeenCalledWith('tool_editor'));
    const cards = Array.from(container.querySelectorAll('.grid button'));
    expect(cards.map((card) => card.querySelector('.name')?.textContent)).toEqual([
      'Colour picker',
      'Text capture (OCR)',
      'Screen ruler',
      'QR code',
      'Image editor',
      'Pin to screen',
      'Image combiner',
      'Video converter',
    ]);
  });

  it('enables only the image editor card', async () => {
    const { container } = render(ToolsPage);
    await waitFor(() => expect(mocks.available).toHaveBeenCalled());
    const cards = Array.from(container.querySelectorAll<HTMLButtonElement>('.grid button'));
    expect(cards.length).toBe(8);
    expect(cards[4].disabled).toBe(false);
    expect(cards.filter((card) => !card.disabled).length).toBe(1);
    expect(screen.getAllByText('Coming in a later version').length).toBe(7);
  });

  it('opens the image editor from the card', async () => {
    render(ToolsPage);
    await waitFor(() => expect(mocks.available).toHaveBeenCalled());
    await fireEvent.click(screen.getByText('Image editor').closest('button') as Element);
    await waitFor(() => expect(mocks.openFile).toHaveBeenCalledTimes(1));
  });

  it('keeps the default chords visible', async () => {
    render(ToolsPage);
    await waitFor(() => expect(mocks.available).toHaveBeenCalled());
    expect(screen.getByText('Ctrl+Shift+C')).not.toBeNull();
    expect(screen.getByText('Ctrl+Shift+T')).not.toBeNull();
    expect(screen.getByText('Ctrl+Shift+M')).not.toBeNull();
    expect(screen.getAllByText('—').length).toBe(5);
  });
});
