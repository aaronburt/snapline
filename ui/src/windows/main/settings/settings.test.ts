import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { get } from 'svelte/store';
import {
  SUBFOLDER_OPTIONS,
  BUNDLED_FFMPEG_PATH,
  imageFormatValue,
  loadSettings,
  pickFolder,
  requestPreview,
  resetSettings,
  settings,
  settingsError,
  toView,
  updateCapture,
  updateGeneral,
  updateHotkeys,
  updateUploads,
} from './settings';
import { settingsDto } from './settingsDto';

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));

vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }));

beforeEach(() => {
  resetSettings();
  mocks.invoke.mockReset().mockResolvedValue({
    kind: 'updated',
    data: { settings: settingsDto() },
  });
});

afterEach(() => {
  vi.useRealTimers();
});

describe('toView', () => {
  it('maps stored values to the labels the pages use', () => {
    const view = toView(settingsDto());
    expect(view.general.imageFormat).toBe('PNG');
    expect(view.general.subfolders).toBe(SUBFOLDER_OPTIONS[0]);
    expect(view.general.copiedLinkFormat).toBe('Direct link');
    expect(view.capture.magnifierZoom).toBe('4×');
    expect(view.capture.monitorMode).toBe('Monitor under cursor');
    expect(view.capture.videoFormat).toBe('MP4 · H.264');
    expect(view.uploads.retry).toBe('3 times');
    expect(view.uploads.timeout).toBe('60 s');
    expect(view.uploads.keepHistory).toBe('Forever');
    expect(view.uploads.proxy).toBe('Use system proxy');
    expect(view.hotkeys.captureRegion).toBe('PrtSc');
    expect(view.hotkeys.takeOverPrtSc).toBe(true);
    expect(view.general.trayClickCapture).toBe(false);
  });

  it('keeps a missing optional path empty', () => {
    const dto = settingsDto();
    dto.capture.ffmpegPath = null;
    dto.uploads.proxyUrl = null;
    expect(toView(dto).capture.ffmpegPath).toBe('');
    expect(toView(dto).uploads.customProxy).toBe('');
  });

  it('uses safe labels and default hotkeys for unknown stored values', () => {
    const dto = settingsDto();
    dto.general.imageFormat = 'future_format' as never;
    dto.capture.captureDelay = 'future_delay' as never;
    dto.capture.videoFormat = 'future_codec' as never;
    dto.uploads.retry = 'future_retry' as never;
    dto.hotkeys.bindings = {};

    const view = toView(dto);

    expect(view.general.imageFormat).toBe('PNG');
    expect(view.capture.captureDelay).toBe('Off');
    expect(view.capture.videoFormat).toBe('MP4 · H.264');
    expect(view.uploads.retry).toBe('3 times');
    expect(view.hotkeys.captureRegion).toBe('PrtSc');
  });
});

describe('imageFormatValue', () => {
  it('maps the displayed format to the stored value', () => {
    expect(imageFormatValue('PNG')).toBe('png');
    expect(imageFormatValue('JPEG')).toBe('jpeg');
    expect(imageFormatValue('WebP')).toBe('webp');
  });
});

describe('resetSettings', () => {
  it('restores the documented defaults', () => {
    settings.update((current) => ({
      ...current,
      general: { ...current.general, notifications: false },
    }));
    resetSettings();
    expect(get(settings).general.notifications).toBe(true);
    expect(get(settings).general.saveFolder).toBe('%USERPROFILE%\\Pictures\\Snapline');
  });
});

describe('settings updates', () => {
  it('encodes displayed enum values and hotkeys into backend settings values', () => {
    mocks.invoke.mockImplementation(() => new Promise(() => {}));

    updateGeneral('copiedLinkFormat', 'Markdown');
    updateCapture('captureDelay', '3 s');
    updateHotkeys('captureRegion', 'Ctrl+Shift+A');
    updateUploads('images', 'Personal uploader');

    expect(mocks.invoke.mock.calls).toEqual([
      [
        'ipc_settings',
        { req: { kind: 'update', data: { patch: { general: { copied_link_format: 'markdown' } } } } },
      ],
      [
        'ipc_settings',
        { req: { kind: 'update', data: { patch: { capture: { capture_delay: '3s' } } } } },
      ],
      [
        'ipc_settings',
        {
          req: {
            kind: 'update',
            data: { patch: { hotkeys: { bindings: { capture_region: 'Ctrl+Shift+A' } } } },
          },
        },
      ],
      [
        'ipc_settings',
        { req: { kind: 'update', data: { patch: { uploads: { images: 'Personal uploader' } } } } },
      ],
    ]);
  });

  it('debounces text fields and combines their patch into one save', async () => {
    vi.useFakeTimers();
    mocks.invoke.mockImplementation(() => new Promise(() => {}));

    updateGeneral('saveFolder', 'D:\\Captures');
    updateGeneral('namePattern', '{app}_{n}');
    expect(mocks.invoke).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(299);
    expect(mocks.invoke).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(1);

    expect(mocks.invoke).toHaveBeenCalledExactlyOnceWith('ipc_settings', {
      req: {
        kind: 'update',
        data: { patch: { general: { save_folder: 'D:\\Captures', name_pattern: '{app}_{n}' } } },
      },
    });
  });

  it('flushes pending text fields when an immediate setting changes', () => {
    vi.useFakeTimers();
    mocks.invoke.mockImplementation(() => new Promise(() => {}));

    updateGeneral('saveFolder', 'D:\\Captures');
    updateGeneral('imageFormat', 'JPEG');

    expect(mocks.invoke).toHaveBeenCalledExactlyOnceWith('ipc_settings', {
      req: {
        kind: 'update',
        data: { patch: { general: { save_folder: 'D:\\Captures', image_format: 'jpeg' } } },
      },
    });
    vi.advanceTimersByTime(300);
    expect(mocks.invoke).toHaveBeenCalledTimes(1);
  });

  it('protects the FFmpeg path from ordinary settings patches', () => {
    updateCapture('ffmpegPath', 'C:\\tools\\ffmpeg.exe');

    expect(mocks.invoke).not.toHaveBeenCalled();
    expect(get(settings).capture.ffmpegPath).toBe(BUNDLED_FFMPEG_PATH);
  });

  it('reports a failed save using the IPC error message', async () => {
    mocks.invoke.mockRejectedValue({ code: 'io', message: 'Settings folder is read-only.' });

    updateGeneral('notifications', false);

    await vi.waitFor(() => expect(get(settingsError)).toBe('Settings folder is read-only.'));
  });
});

describe('settings loading helpers', () => {
  it('loads current settings and clears an earlier error', async () => {
    settingsError.set('previous error');
    mocks.invoke.mockResolvedValue({ kind: 'settings', data: { settings: settingsDto() } });

    await loadSettings();

    expect(get(settings).general.imageFormat).toBe('PNG');
    expect(get(settingsError)).toBeNull();
    expect(mocks.invoke).toHaveBeenCalledWith('ipc_settings', { req: { kind: 'get' } });
  });

  it('sends an empty current folder as null and returns preview and picker results', async () => {
    mocks.invoke
      .mockResolvedValueOnce({ kind: 'previewName', data: { name: 'screen_1.png' } })
      .mockResolvedValueOnce({ kind: 'browseFolder', data: { path: 'D:\\Pictures' } });

    await expect(requestPreview('{app}_{n}', 'png')).resolves.toBe('screen_1.png');
    await expect(pickFolder('')).resolves.toBe('D:\\Pictures');
    expect(mocks.invoke).toHaveBeenNthCalledWith(1, 'ipc_settings', {
      req: { kind: 'previewName', data: { pattern: '{app}_{n}', format: 'png' } },
    });
    expect(mocks.invoke).toHaveBeenNthCalledWith(2, 'ipc_settings', {
      req: { kind: 'browseFolder', data: { current: null } },
    });
  });

  it('returns safe fallbacks and stores errors when preview or folder selection fails', async () => {
    mocks.invoke.mockRejectedValueOnce('preview unavailable').mockRejectedValueOnce('picker unavailable');

    await expect(requestPreview('{app}', 'png')).resolves.toBe('');
    expect(get(settingsError)).toBe('preview unavailable');
    await expect(pickFolder('D:\\Pictures')).resolves.toBeNull();
    expect(get(settingsError)).toBe('picker unavailable');
  });
});
