import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { expectAccessible } from '../../../test/axe';
import type { HistoryCaptureDto } from '../../../lib/ipc/bindings/HistoryCaptureDto';
import ConverterTool from './ConverterTool.svelte';

const ipc = vi.hoisted(() => ({
  convertPickFile: vi.fn(),
  convertStart: vi.fn(),
  convertProgress: vi.fn(),
  convertCancel: vi.fn(),
  convertResult: vi.fn(),
}));

const history = vi.hoisted(() => ({
  historyPage: vi.fn(),
}));

vi.mock('../../../lib/ipc/tool', () => ipc);
vi.mock('../../../lib/ipc/history', () => history);

function capture(id: number, overrides: Partial<HistoryCaptureDto> = {}): HistoryCaptureDto {
  return {
    id,
    kind: 'video',
    name: `recording-${id}.mp4`,
    bytes: 4 * 1024 * 1024,
    width: 1920,
    height: 1080,
    durationMs: 42_000,
    sourceApp: 'chrome.exe',
    createdAt: Date.now(),
    folder: 'Pictures\\Snapline\\2026-11',
    upload: null,
    ...overrides,
  };
}

function page(items: HistoryCaptureDto[]) {
  return { items, next: null };
}

beforeEach(() => {
  ipc.convertPickFile.mockReset();
  ipc.convertStart.mockReset();
  ipc.convertProgress.mockReset();
  ipc.convertCancel.mockReset();
  ipc.convertResult.mockReset();
  history.historyPage.mockReset();
  history.historyPage.mockResolvedValue(page([capture(1), capture(2)]));
  ipc.convertPickFile.mockResolvedValue('C:\\Videos\\clip.mp4');
  ipc.convertStart.mockResolvedValue(7);
  ipc.convertProgress.mockResolvedValue({
    phase: 'converting',
    percent: 25,
    outTimeMs: 1_500,
    speed: 1.5,
    message: null,
  });
  ipc.convertCancel.mockResolvedValue(undefined);
  ipc.convertResult.mockResolvedValue({
    captureId: 12,
    path: 'C:\\Videos\\recording-1.gif',
    name: 'recording-1.gif',
    format: 'gif',
  });
});

afterEach(() => {
  cleanup();
});

describe('ConverterTool', () => {
  it('shows History recordings and selects the first one', async () => {
    const { container } = render(ConverterTool);
    await waitFor(() => expect(history.historyPage).toHaveBeenCalledWith('videos', null));
    await expectAccessible(container);
    expect(screen.getByText('Video converter')).not.toBeNull();
    expect(screen.getByText('Turn recordings into GIF, WebM or a smaller MP4')).not.toBeNull();
    await waitFor(() => expect(history.historyPage).toHaveBeenCalledWith('videos', null));
    expect(await screen.findByText('recording-1.mp4')).not.toBeNull();
    expect(screen.getByText('recording-2.mp4')).not.toBeNull();
    const first = screen.getByRole('button', { name: /recording-1\.mp4/ });
    expect(first.getAttribute('aria-pressed')).toBe('true');
    expect(screen.getByRole('combobox', { name: 'Output format' })).not.toBeNull();
  });

  it('starts a GIF conversion from the selected capture and shows progress', async () => {
    render(ConverterTool);
    await waitFor(() => expect(history.historyPage).toHaveBeenCalled());
    await fireEvent.click(screen.getByRole('button', { name: 'Convert' }));
    await waitFor(() =>
      expect(ipc.convertStart).toHaveBeenCalledWith(
        { kind: 'capture', data: { captureId: 1 } },
        'gif',
      ),
    );
    await waitFor(() => expect(ipc.convertProgress).toHaveBeenCalledWith(7));
    const bar = screen.getByRole('progressbar', { name: 'Conversion progress' });
    expect(bar.getAttribute('aria-valuenow')).toBe('25');
    expect(screen.getByText('25%')).not.toBeNull();
  });

  it('chooses a file, converts to WebM and shows the History result', async () => {
    ipc.convertProgress.mockResolvedValue({
      phase: 'done',
      percent: 100,
      outTimeMs: null,
      speed: null,
      message: null,
    });
    render(ConverterTool);
    await waitFor(() => expect(history.historyPage).toHaveBeenCalled());
    await fireEvent.click(screen.getByRole('radio', { name: 'File' }));
    await fireEvent.click(screen.getByRole('button', { name: 'Choose video' }));
    await waitFor(() => expect(ipc.convertPickFile).toHaveBeenCalledTimes(1));
    expect(await screen.findByText('C:\\Videos\\clip.mp4')).not.toBeNull();
    const select = screen.getByRole('combobox', { name: 'Output format' }) as HTMLSelectElement;
    await fireEvent.change(select, { target: { value: 'WebM (VP9)' } });
    await fireEvent.click(screen.getByRole('button', { name: 'Convert' }));
    await waitFor(() =>
      expect(ipc.convertStart).toHaveBeenCalledWith(
        { kind: 'file', data: { path: 'C:\\Videos\\clip.mp4' } },
        'webmVp9',
      ),
    );
    expect(await screen.findByText('Saved recording-1.gif')).not.toBeNull();
    const link = screen.getByRole('link', { name: 'Open in History' });
    expect(link.getAttribute('href')).toBe('#/history');
  });

  it('cancels a running conversion', async () => {
    ipc.convertProgress
      .mockResolvedValueOnce({
        phase: 'converting',
        percent: 10,
        outTimeMs: 1_000,
        speed: 1,
        message: null,
      })
      .mockResolvedValue({
        phase: 'cancelled',
        percent: null,
        outTimeMs: null,
        speed: null,
        message: 'The conversion was cancelled.',
      });
    render(ConverterTool);
    await waitFor(() => expect(history.historyPage).toHaveBeenCalled());
    await fireEvent.click(screen.getByRole('button', { name: 'Convert' }));
    await waitFor(() => expect(ipc.convertProgress).toHaveBeenCalledWith(7));
    await fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    await waitFor(() => expect(ipc.convertCancel).toHaveBeenCalledWith(7));
    expect(await screen.findByText('Cancelled')).not.toBeNull();
    expect(screen.queryByRole('button', { name: 'Cancel' })).toBeNull();
  });

  it('shows the service message when the converter is not available', async () => {
    ipc.convertStart.mockRejectedValue({
      code: 'internal',
      message: 'FFmpeg is not available.',
    });
    render(ConverterTool);
    await waitFor(() => expect(history.historyPage).toHaveBeenCalled());
    await fireEvent.click(screen.getByRole('button', { name: 'Convert' }));
    const alert = await screen.findByRole('alert');
    expect(alert.textContent).toContain('FFmpeg is not available.');
  });

  it('shows the message when a file cannot be chosen', async () => {
    ipc.convertPickFile.mockRejectedValue({
      code: 'internal',
      message: 'The file picker is not available.',
    });
    render(ConverterTool);
    await waitFor(() => expect(history.historyPage).toHaveBeenCalled());
    await fireEvent.click(screen.getByRole('radio', { name: 'File' }));
    await fireEvent.click(screen.getByRole('button', { name: 'Choose video' }));
    const alert = await screen.findByRole('alert');
    expect(alert.textContent).toContain('The file picker is not available.');
  });
});
