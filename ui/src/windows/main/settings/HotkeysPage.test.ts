import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import HotkeysPage from './HotkeysPage.svelte';
import { resetSettings } from './settings';
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

describe('HotkeysPage', () => {
  it('gets the settings on mount and shows the page intro', async () => {
    render(HotkeysPage);
    expect(
      screen.getByText(
        'If two actions share a chord, or another app owns it, neither is bound and you are told.',
      ),
    ).not.toBeNull();
    await waitFor(() => expect(ipc.get).toHaveBeenCalledTimes(1));
  });

  it('shows the section 7.5 default chords', () => {
    render(HotkeysPage);
    const defaults: Record<string, string> = {
      'Capture region': 'PrtSc',
      'Capture window': 'Alt+PrtSc',
      'Capture full screen': 'Ctrl+PrtSc',
      'Scrolling capture': 'Ctrl+Alt+S',
      'Last region again': 'Ctrl+Shift+R',
      'Record video': 'Shift+PrtSc',
      'Record GIF': 'Ctrl+Shift+G',
      'Text capture (OCR)': 'Ctrl+Shift+T',
      'Colour picker': 'Ctrl+Shift+C',
      'Screen ruler': 'Ctrl+Shift+M',
      'Upload from clipboard': 'Ctrl+Shift+V',
    };
    for (const [label, chord] of Object.entries(defaults)) {
      expect((screen.getByLabelText(label) as HTMLInputElement).value).toBe(chord);
    }
  });

  it('shows the behaviour toggles', () => {
    render(HotkeysPage);
    expect(screen.getByText('Take over the PrtSc key')).not.toBeNull();
    expect(screen.getByText('Warn about clashes')).not.toBeNull();
    expect(
      (screen.getByRole('checkbox', { name: /Take over the PrtSc key/ }) as HTMLInputElement).checked,
    ).toBe(true);
    expect(
      (screen.getByRole('checkbox', { name: /Warn about clashes/ }) as HTMLInputElement).checked,
    ).toBe(true);
  });

  it('sends a behaviour change to the service', async () => {
    render(HotkeysPage);
    await fireEvent.click(screen.getByRole('checkbox', { name: /Warn about clashes/ }));
    await waitFor(() =>
      expect(ipc.update).toHaveBeenCalledWith({ hotkeys: { warn_about_clashes: false } }),
    );
  });
});
