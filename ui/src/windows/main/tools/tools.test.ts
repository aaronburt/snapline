import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { expectAccessible } from '../../../test/axe';
import ToolsPage from './ToolsPage.svelte';

const mocks = vi.hoisted(() => ({
  available: vi.fn(),
  openFile: vi.fn(),
  pinMenu: vi.fn(),
}));

vi.mock('../../../lib/ipc/tool', () => ({
  toolAvailable: mocks.available,
  editorOpenFile: mocks.openFile,
  pinMenu: mocks.pinMenu,
}));

beforeEach(() => {
  mocks.available.mockReset();
  mocks.openFile.mockReset();
  mocks.pinMenu.mockReset();
  mocks.available.mockResolvedValue(true);
  mocks.openFile.mockResolvedValue({
    action: null,
    captureId: null,
    path: null,
    cancelled: true,
  });
  mocks.pinMenu.mockResolvedValue('file');
});

afterEach(cleanup);

describe('Tools page', () => {
  it('renders the eight cards in order', async () => {
    const { container } = render(ToolsPage);
    await waitFor(() => expect(mocks.available).toHaveBeenCalledWith('tool_editor'));
    await expectAccessible(container);
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

  it('enables only the image editor and pin cards', async () => {
    const { container } = render(ToolsPage);
    const pin = screen.getByText('Pin to screen').closest('button') as HTMLButtonElement;
    await waitFor(() => expect(pin.disabled).toBe(false));
    const cards = Array.from(container.querySelectorAll<HTMLButtonElement>('.grid button'));
    expect(cards.length).toBe(8);
    expect(cards[4].disabled).toBe(false);
    expect(cards[5].disabled).toBe(false);
    expect(cards.filter((card) => !card.disabled).length).toBe(2);
    expect(screen.getAllByText('Coming in a later version').length).toBe(6);
  });

  it('shows the pin card description from the design', async () => {
    render(ToolsPage);
    expect(await screen.findByText('Keep an image floating above other windows.')).not.toBeNull();
  });

  it('keeps the pin card parked when the ports are missing', async () => {
    mocks.available.mockImplementation(async (tool: string) => tool === 'tool_editor');
    render(ToolsPage);
    await waitFor(() => expect(mocks.available).toHaveBeenCalledWith('tool_pin'));
    const pin = screen.getByText('Pin to screen').closest('button') as HTMLButtonElement;
    expect(pin.disabled).toBe(true);
    await fireEvent.click(pin);
    expect(mocks.pinMenu).not.toHaveBeenCalled();
  });

  it('opens the image editor from the card', async () => {
    render(ToolsPage);
    const card = screen.getByText('Image editor').closest('button') as HTMLButtonElement;
    await waitFor(() => expect(card.disabled).toBe(false));
    await fireEvent.click(card);
    await waitFor(() => expect(mocks.openFile).toHaveBeenCalledTimes(1));
  });

  it('opens the native pin menu from the card', async () => {
    render(ToolsPage);
    const pin = screen.getByText('Pin to screen').closest('button') as HTMLButtonElement;
    await waitFor(() => expect(pin.disabled).toBe(false));
    await fireEvent.click(pin);
    await waitFor(() => expect(mocks.pinMenu).toHaveBeenCalledTimes(1));
  });

  it('shows the pin error when the native menu cannot pin', async () => {
    mocks.pinMenu.mockRejectedValue({
      code: 'invalid',
      message: 'The clipboard does not contain an image.',
    });
    render(ToolsPage);
    const pin = screen.getByText('Pin to screen').closest('button') as HTMLButtonElement;
    await waitFor(() => expect(pin.disabled).toBe(false));
    await fireEvent.click(pin);
    await waitFor(() =>
      expect(screen.getByRole('status').textContent).toContain(
        'The clipboard does not contain an image.',
      ),
    );
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
