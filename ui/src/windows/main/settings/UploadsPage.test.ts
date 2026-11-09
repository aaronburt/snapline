import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import UploadsPage from './UploadsPage.svelte';
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

describe('UploadsPage', () => {
  it('gets the settings on mount and defaults every destination', async () => {
    render(UploadsPage);
    for (const title of ['Images', 'Videos and GIFs', 'Text and code', 'Other files']) {
      expect((screen.getByRole('combobox', { name: title }) as HTMLSelectElement).value).toBe(
        'Save locally only',
      );
    }
    expect(
      (screen.getByRole('combobox', { name: 'Link shortener' }) as HTMLSelectElement).value,
    ).toBe('None');
    await waitFor(() => expect(ipc.get).toHaveBeenCalledTimes(1));
  });

  it('shows the account buttons and the empty destination list', () => {
    render(UploadsPage);
    expect(screen.getByRole('button', { name: /Import ShareX uploader \(\.sxcu\)/ })).not.toBeNull();
    expect(screen.getByRole('button', { name: /Add destination/ })).not.toBeNull();
    expect(screen.getByText('No destinations yet')).not.toBeNull();
  });

  it('shows the upload behaviour defaults', () => {
    render(UploadsPage);
    expect(
      (screen.getByRole('combobox', { name: 'Retry failed uploads' }) as HTMLSelectElement).value,
    ).toBe('3 times');
    expect(screen.getByRole('radio', { name: '60 s' }).getAttribute('aria-checked')).toBe('true');
    expect(
      (screen.getByRole('combobox', { name: 'Keep upload history' }) as HTMLSelectElement).value,
    ).toBe('Forever');
    expect((screen.getByRole('combobox', { name: 'Proxy' }) as HTMLSelectElement).value).toBe(
      'Use system proxy',
    );
  });

  it('sends a select change to the service', async () => {
    render(UploadsPage);
    await fireEvent.change(screen.getByRole('combobox', { name: 'Retry failed uploads' }), {
      target: { value: 'Once' },
    });
    await waitFor(() =>
      expect(ipc.update).toHaveBeenCalledWith({ uploads: { retry: 'once' } }),
    );
  });
});
