import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
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

const hotkeysIpc = vi.hoisted(() => ({
  ping: vi.fn(),
  get: vi.fn(),
  preview: vi.fn(),
  pause: vi.fn(),
  resume: vi.fn(),
}));

vi.mock('../../../lib/ipc/settings', () => ipc);
vi.mock('../../../lib/ipc/hotkeys', () => hotkeysIpc);

const DEFAULTS: Array<[string, string, string]> = [
  ['capture_region', 'Capture region', 'PrtSc'],
  ['capture_window', 'Capture window', 'Alt+PrtSc'],
  ['capture_screen', 'Capture full screen', 'Ctrl+PrtSc'],
  ['capture_scrolling', 'Scrolling capture', 'Ctrl+Alt+S'],
  ['capture_last_region', 'Last region again', 'Ctrl+Shift+R'],
  ['record_video', 'Record video', 'Shift+PrtSc'],
  ['record_gif', 'Record GIF', 'Ctrl+Shift+G'],
  ['tool_ocr', 'Text capture (OCR)', 'Ctrl+Shift+T'],
  ['tool_colour', 'Colour picker', 'Ctrl+Shift+C'],
  ['tool_ruler', 'Screen ruler', 'Ctrl+Shift+M'],
  ['upload_clipboard', 'Upload from clipboard', 'Ctrl+Shift+V'],
];

const PRTSC_HINT = 'Sign out and back in for Windows to release the PrtSc key.';

function bindings() {
  return DEFAULTS.map(([owner, label, chord]) => ({ owner, label, chord }));
}

function state() {
  return { bindings: bindings(), blocked: [], paused: false };
}

function row(owner: string): HTMLElement {
  const element = document.querySelector(`[data-owner="${owner}"]`);
  if (!(element instanceof HTMLElement)) {
    throw new Error(`missing row ${owner}`);
  }
  return element;
}

async function begin(owner: string): Promise<void> {
  await fireEvent.click(within(row(owner)).getByRole('button', { name: 'Change' }));
}

beforeEach(() => {
  resetSettings();
  ipc.get.mockReset();
  ipc.update.mockReset();
  ipc.previewName.mockReset();
  ipc.browseFolder.mockReset();
  hotkeysIpc.get.mockReset();
  hotkeysIpc.preview.mockReset();
  ipc.get.mockResolvedValue(settingsDto());
  ipc.update.mockResolvedValue(settingsDto());
  hotkeysIpc.get.mockResolvedValue(state());
  hotkeysIpc.preview.mockResolvedValue('free');
});

afterEach(cleanup);

describe('HotkeysPage', () => {
  it('gets the settings and hotkey bindings on mount and shows the page intro', async () => {
    render(HotkeysPage);
    expect(
      screen.getByText(
        'If two actions share a chord, or another app owns it, neither is bound and you are told.',
      ),
    ).not.toBeNull();
    await waitFor(() => expect(ipc.get).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(hotkeysIpc.get).toHaveBeenCalledTimes(1));
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

  it('shows the live chord reported by the hotkeys service', async () => {
    hotkeysIpc.get.mockResolvedValue({
      bindings: bindings().map((binding) =>
        binding.owner === 'capture_region' ? { ...binding, chord: 'Ctrl+Alt+P' } : binding,
      ),
      blocked: [],
      paused: false,
    });
    render(HotkeysPage);
    await waitFor(() =>
      expect((screen.getByLabelText('Capture region') as HTMLInputElement).value).toBe(
        'Ctrl+Alt+P',
      ),
    );
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

  it('previews an internal clash while capturing and shows the other action', async () => {
    hotkeysIpc.preview.mockResolvedValue({ internal: { owners: ['record_gif'] } });
    render(HotkeysPage);
    await begin('capture_region');
    await fireEvent.keyDown(window, { key: 'g', ctrlKey: true, shiftKey: true });
    await waitFor(() =>
      expect(hotkeysIpc.preview).toHaveBeenCalledWith('capture_region', 'Ctrl+Shift+G'),
    );
    expect(
      await within(row('capture_region')).findByText('Off: also used by Record GIF.'),
    ).not.toBeNull();
    expect(row('capture_region').classList.contains('blocked')).toBe(true);
  });

  it('previews an external clash while capturing', async () => {
    hotkeysIpc.preview.mockResolvedValue('external');
    render(HotkeysPage);
    await begin('capture_region');
    await fireEvent.keyDown(window, { key: 'F5' });
    await waitFor(() =>
      expect(hotkeysIpc.preview).toHaveBeenCalledWith('capture_region', 'F5'),
    );
    expect(
      await within(row('capture_region')).findByText(
        'Off: another app is using this shortcut.',
      ),
    ).not.toBeNull();
  });

  it('previews a free chord without clash text', async () => {
    hotkeysIpc.preview.mockResolvedValue('free');
    render(HotkeysPage);
    await begin('capture_region');
    await fireEvent.keyDown(window, { key: 'F5' });
    await waitFor(() =>
      expect(hotkeysIpc.preview).toHaveBeenCalledWith('capture_region', 'F5'),
    );
    expect(within(row('capture_region')).queryByText(/Off:/)).toBeNull();
  });

  it('marks rows that the service reports as blocked', async () => {
    hotkeysIpc.get.mockResolvedValue({
      bindings: bindings().filter(
        (binding) =>
          binding.owner !== 'capture_region' && binding.owner !== 'capture_window',
      ),
      blocked: [
        {
          owner: 'capture_region',
          label: 'Capture region',
          chord: 'Ctrl+Shift+G',
          reason: 'internal',
        },
        {
          owner: 'record_gif',
          label: 'Record GIF',
          chord: 'Ctrl+Shift+G',
          reason: 'internal',
        },
        {
          owner: 'capture_window',
          label: 'Capture window',
          chord: 'Alt+PrtSc',
          reason: 'external',
        },
      ],
      paused: false,
    });
    render(HotkeysPage);
    expect(
      await within(row('capture_region')).findByText('Off: also used by Record GIF.'),
    ).not.toBeNull();
    expect(
      await within(row('capture_window')).findByText(
        'Off: another app is using this shortcut.',
      ),
    ).not.toBeNull();
  });

  it('sends the new chord and refreshes the bindings', async () => {
    render(HotkeysPage);
    await begin('record_gif');
    await fireEvent.keyDown(window, { key: 'g', ctrlKey: true, altKey: true });
    await waitFor(() =>
      expect(ipc.update).toHaveBeenCalledWith({
        hotkeys: { bindings: { record_gif: 'Ctrl+Alt+G' } },
      }),
    );
    await waitFor(() => expect(hotkeysIpc.get.mock.calls.length).toBeGreaterThan(1));
  });

  it('shows the sign-out hint the first time takeover is enabled', async () => {
    ipc.update.mockImplementation(async (patch: { hotkeys?: { take_over_prtsc?: boolean } }) => {
      const next = settingsDto();
      if (typeof patch.hotkeys?.take_over_prtsc === 'boolean') {
        next.hotkeys.takeOverPrtsc = patch.hotkeys.take_over_prtsc;
      }
      return next;
    });
    render(HotkeysPage);
    await waitFor(() => expect(ipc.get).toHaveBeenCalledTimes(1));
    expect(screen.queryByText(PRTSC_HINT)).toBeNull();
    const toggle = screen.getByRole('checkbox', { name: /Take over the PrtSc key/ });
    await fireEvent.click(toggle);
    expect(screen.queryByText(PRTSC_HINT)).toBeNull();
    await fireEvent.click(toggle);
    expect(await screen.findByText(PRTSC_HINT)).not.toBeNull();
    await fireEvent.click(toggle);
    await fireEvent.click(toggle);
    await waitFor(() => expect(screen.queryByText(PRTSC_HINT)).toBeNull());
  });
});
