import { beforeEach, describe, expect, it } from 'vitest';
import { get } from 'svelte/store';
import { SUBFOLDER_OPTIONS, imageFormatValue, resetSettings, settings, toView } from './settings';
import { settingsDto } from './settingsDto';

beforeEach(resetSettings);

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
