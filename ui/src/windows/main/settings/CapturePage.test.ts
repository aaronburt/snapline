import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { expectAccessible } from '../../../test/axe';
import CapturePage from './CapturePage.svelte';
import { BUNDLED_FFMPEG_PATH, resetSettings } from './settings';
import { settingsDto } from './settingsDto';

const ipc = vi.hoisted(() => ({
  ping: vi.fn(),
  get: vi.fn(),
  update: vi.fn(),
  previewName: vi.fn(),
  browseFolder: vi.fn(),
}));

vi.mock('../../../lib/ipc/settings', () => ipc);

beforeEach(() => {
  resetSettings();
  ipc.get.mockReset();
  ipc.update.mockReset();
  ipc.get.mockResolvedValue(settingsDto());
  ipc.update.mockResolvedValue(settingsDto());
});

afterEach(cleanup);

describe('CapturePage', () => {
  it('gets the settings on mount and renders every section', async () => {
    const { container } = render(CapturePage);
    await expectAccessible(container);
    for (const title of [
      'Screenshots',
      'Region selection',
      'Monitors',
      'Screen recording',
      'GIF',
      'Scrolling capture',
    ]) {
      expect(screen.getByText(title)).not.toBeNull();
    }
    await waitFor(() => expect(ipc.get).toHaveBeenCalledTimes(1));
  });

  it('shows the documented defaults', () => {
    render(CapturePage);
    expect(
      (screen.getByRole('checkbox', { name: /Include mouse cursor/ }) as HTMLInputElement).checked,
    ).toBe(false);
    expect(screen.getByRole('radio', { name: 'Off' }).getAttribute('aria-checked')).toBe('true');
    expect(
      (screen.getByRole('checkbox', { name: /Shutter sound/ }) as HTMLInputElement).checked,
    ).toBe(true);
    expect(screen.getByRole('radio', { name: '4×' }).getAttribute('aria-checked')).toBe('true');
    expect(
      (screen.getByRole('combobox', { name: 'Full-screen capture uses' }) as HTMLSelectElement).value,
    ).toBe('Monitor under cursor');
    expect(
      (screen.getByRole('combobox', { name: 'Video format' }) as HTMLSelectElement).value,
    ).toBe('MP4 · H.264');
    expect(
      screen
        .getAllByRole('radio', { name: '30' })
        .filter((radio) => radio.getAttribute('aria-checked') === 'true'),
    ).toHaveLength(1);
    expect(screen.getByRole('radio', { name: '15' }).getAttribute('aria-checked')).toBe('true');
    expect(
      (screen.getByLabelText('Microphone device') as HTMLSelectElement).value,
    ).toBe('Default microphone');
    expect(screen.getByText('Default microphone')).not.toBeNull();
    expect((screen.getByLabelText('FFmpeg location') as HTMLInputElement).value).toBe(
      BUNDLED_FFMPEG_PATH,
    );
    expect(screen.getByText('300 ms')).not.toBeNull();
    expect(screen.getByText('The monitor with the cursor is captured')).not.toBeNull();
  });

  it('lists only the placeholder microphone', () => {
    render(CapturePage);
    const select = screen.getByLabelText('Microphone device') as HTMLSelectElement;
    expect(select.options).toHaveLength(1);
    expect(select.options[0].text).toBe('Default microphone');
  });

  it('sends a toggle and a segment change to the service', async () => {
    render(CapturePage);
    await fireEvent.click(screen.getByRole('checkbox', { name: /Include mouse cursor/ }));
    await waitFor(() =>
      expect(ipc.update).toHaveBeenCalledWith({ capture: { include_cursor: true } }),
    );
    await fireEvent.click(screen.getByRole('radio', { name: '8×' }));
    await waitFor(() =>
      expect(ipc.update).toHaveBeenCalledWith({ capture: { magnifier_zoom: '8x' } }),
    );
  });
});
