import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
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
const displayIpc = vi.hoisted(() => ({ monitors: vi.fn() }));

vi.mock('../../../lib/ipc/settings', () => ipc);
vi.mock('../../../lib/ipc/app', () => displayIpc);

const displays = [
  { id: 81, name: '\\\\.\\DISPLAY1', x: -1920, y: 360, width: 1920, height: 1080, primary: false },
  { id: 84, name: '\\\\.\\DISPLAY4', x: 0, y: 0, width: 2560, height: 1440, primary: true },
];

beforeEach(() => {
  resetSettings();
  ipc.get.mockReset();
  ipc.update.mockReset();
  ipc.get.mockResolvedValue(settingsDto());
  ipc.update.mockResolvedValue(settingsDto());
  displayIpc.monitors.mockReset();
  displayIpc.monitors.mockResolvedValue(displays);
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
    ]) {
      expect(screen.getByText(title)).not.toBeNull();
    }
    await waitFor(() => expect(ipc.get).toHaveBeenCalledTimes(1));
  });

  it('shows the documented defaults', async () => {
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
      (screen.getByRole('combobox', { name: 'Full-screen capture uses' }) as HTMLSelectElement)
        .value,
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
    expect((screen.getByLabelText('Microphone device') as HTMLSelectElement).value).toBe(
      'Default microphone',
    );
    expect(screen.getByText('Default microphone')).not.toBeNull();
    expect((screen.getByLabelText('FFmpeg location') as HTMLInputElement).value).toBe(
      BUNDLED_FFMPEG_PATH,
    );
    expect(await screen.findByText('The monitor with the cursor is captured')).not.toBeNull();
  });

  it('shows the detected desktop bounds and highlights the real primary display', async () => {
    const settings = settingsDto();
    settings.capture.fullscreenCaptureUses = 'all_monitors';
    ipc.get.mockResolvedValue(settings);
    render(CapturePage);

    expect(
      await screen.findByText('All monitors are captured as one 4480 × 1440 image'),
    ).not.toBeNull();
    const diagram = screen.getByRole('img', { name: /Monitor 1.*Monitor 4.*primary/ });
    const left = within(diagram).getByText('1').parentElement!;
    const primary = within(diagram).getByText('4').parentElement!;
    expect(left.parentElement!.style.top).toBe('25%');
    expect(Number.parseFloat(primary.parentElement!.style.left)).toBeCloseTo(42.857);
    expect(left.classList.contains('on')).toBe(true);

    settings.capture.fullscreenCaptureUses = 'primary_monitor';
    ipc.update.mockResolvedValue(settings);
    await fireEvent.change(screen.getByRole('combobox', { name: 'Full-screen capture uses' }), {
      target: { value: 'Primary monitor' },
    });
    expect(await screen.findByText('Monitor 4 (primary) is captured')).not.toBeNull();
    expect(left.classList.contains('on')).toBe(false);
    expect(primary.classList.contains('on')).toBe(true);
  });

  it('refreshes the layout on focus and handles an unavailable monitor list', async () => {
    displayIpc.monitors.mockRejectedValueOnce(new Error('Unavailable'));
    render(CapturePage);
    expect(await screen.findByText('Monitor layout is unavailable.')).not.toBeNull();
    await fireEvent.focus(window);
    expect(await screen.findByRole('img', { name: /Monitor 1.*Monitor 4/ })).not.toBeNull();
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
