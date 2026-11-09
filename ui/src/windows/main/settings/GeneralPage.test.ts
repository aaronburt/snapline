import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import GeneralPage from './GeneralPage.svelte';
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
  ipc.previewName.mockReset();
  ipc.browseFolder.mockReset();
  ipc.get.mockResolvedValue(settingsDto());
  ipc.update.mockResolvedValue(settingsDto());
  ipc.previewName.mockResolvedValue('chrome_2026-11-08_14-31-07.png');
  ipc.browseFolder.mockResolvedValue(null);
});

afterEach(cleanup);

describe('GeneralPage', () => {
  it('gets the settings on mount and shows the defaults', async () => {
    render(GeneralPage);
    expect(screen.getByText('General')).not.toBeNull();
    expect(screen.getByText('Snapline')).not.toBeNull();
    expect(screen.getByText('Version 0.1.0')).not.toBeNull();
    expect(screen.getByText('Launch at sign-in')).not.toBeNull();
    expect(screen.getByText('Save captures to')).not.toBeNull();
    expect((screen.getByLabelText('Save captures to') as HTMLInputElement).value).toBe(
      '%USERPROFILE%\\Pictures\\Snapline',
    );
    expect(screen.getByRole('radio', { name: 'PNG' }).getAttribute('aria-checked')).toBe('true');
    expect(screen.getByText('90%')).not.toBeNull();
    await waitFor(() => expect(ipc.get).toHaveBeenCalledTimes(1));
  });

  it('shows the name pattern preview from the service', async () => {
    render(GeneralPage);
    expect(await screen.findByText('chrome_2026-11-08_14-31-07.png')).not.toBeNull();
    expect(ipc.previewName).toHaveBeenCalledWith(
      '{app}_{yyyy}-{MM}-{dd}_{HH}-{mm}-{ss}',
      'png',
    );
  });

  it('sends a text change after the 300 ms debounce', async () => {
    render(GeneralPage);
    await fireEvent.input(screen.getByLabelText('Name pattern'), {
      target: { value: '{app}_shot' },
    });
    await waitFor(() =>
      expect(ipc.update).toHaveBeenCalledWith({ general: { name_pattern: '{app}_shot' } }),
    );
  });

  it('sends a toggle change immediately', async () => {
    render(GeneralPage);
    await fireEvent.click(screen.getByRole('checkbox', { name: /Launch at sign-in/ }));
    await waitFor(() =>
      expect(ipc.update).toHaveBeenCalledWith({ general: { launch_at_signin: false } }),
    );
  });

  it('inserts a token into the pattern', async () => {
    render(GeneralPage);
    await fireEvent.click(screen.getByRole('button', { name: '{n}' }));
    expect((screen.getByLabelText('Name pattern') as HTMLInputElement).value).toBe(
      '{app}_{yyyy}-{MM}-{dd}_{HH}-{mm}-{ss}{n}',
    );
    await waitFor(() => expect(ipc.update).toHaveBeenCalled());
  });

  it('asks the service for a folder and applies the pick', async () => {
    ipc.browseFolder.mockResolvedValue('C:\\Captures');
    render(GeneralPage);
    await fireEvent.click(screen.getByRole('button', { name: 'Browse…' }));
    await waitFor(() =>
      expect(ipc.update).toHaveBeenCalledWith({ general: { save_folder: 'C:\\Captures' } }),
    );
  });

  it('filters rows by description on the current page', () => {
    render(GeneralPage, { props: { filter: 'Organise saved files automatically' } });
    expect(screen.getByText('Subfolders')).not.toBeNull();
    expect(screen.queryByText('Launch at sign-in')).toBeNull();
  });
});
