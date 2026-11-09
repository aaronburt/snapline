import { writable } from 'svelte/store';
import type { IpcError } from '../../../lib/ipc/core';
import type { SettingsDto } from '../../../lib/ipc/bindings/SettingsDto';
import * as ipc from '../../../lib/ipc/settings';

export interface WorkflowStepSettings {
  kind: string;
  enabled: boolean;
}

export interface GeneralSettings {
  launchAtSignIn: boolean;
  startMinimised: boolean;
  notifications: boolean;
  saveFolder: string;
  subfolders: string;
  imageFormat: string;
  lossyQuality: number;
  namePattern: string;
  defaultWorkflow: WorkflowStepSettings[];
  copiedLinkFormat: string;
}

export interface CaptureSettings {
  includeCursor: boolean;
  captureDelay: string;
  shutterSound: boolean;
  windowShadow: boolean;
  freezeScreen: boolean;
  magnifier: boolean;
  magnifierZoom: string;
  snapWindows: boolean;
  showSize: boolean;
  crosshair: boolean;
  monitorMode: string;
  videoFormat: string;
  frameRate: string;
  quality: string;
  systemAudio: boolean;
  microphone: boolean;
  microphoneDevice: string;
  recordingBorder: boolean;
  ffmpegPath: string;
  gifFrameRate: string;
  gifWidth: string;
  optimisePalette: boolean;
  scrollDelay: number;
  maxScrolls: string;
  removeStickyHeaders: boolean;
}

export interface HotkeySettings {
  captureRegion: string;
  captureWindow: string;
  captureFullScreen: string;
  scrollingCapture: string;
  lastRegion: string;
  recordVideo: string;
  recordGif: string;
  textCapture: string;
  colourPicker: string;
  screenRuler: string;
  uploadClipboard: string;
  takeOverPrtSc: boolean;
  warnClashes: boolean;
}

export interface UploadsSettings {
  images: string;
  videos: string;
  text: string;
  otherFiles: string;
  linkShortener: string;
  removeMetadata: boolean;
  askClipboard: boolean;
  retry: string;
  timeout: string;
  keepHistory: string;
  proxy: string;
  customProxy: string;
}

export interface Settings {
  general: GeneralSettings;
  capture: CaptureSettings;
  hotkeys: HotkeySettings;
  uploads: UploadsSettings;
}

export const IMAGE_FORMAT_OPTIONS = ['PNG', 'JPEG', 'WebP'];
export const SUBFOLDER_OPTIONS = [
  `By month (${new Date().getFullYear()}-${String(new Date().getMonth() + 1).padStart(2, '0')})`,
  `By day (${new Date().getFullYear()}-${String(new Date().getMonth() + 1).padStart(2, '0')}-${String(new Date().getDate()).padStart(2, '0')})`,
  'By app (chrome)',
  'No subfolders',
];
export const LINK_FORMAT_OPTIONS = ['Direct link', 'Markdown', 'HTML <img>', 'BBCode', 'Page link'];
export const CAPTURE_DELAY_OPTIONS = ['Off', '1 s', '3 s', '5 s'];
export const MAGNIFIER_ZOOM_OPTIONS = ['2×', '4×', '8×'];
export const MONITOR_OPTIONS = ['All monitors', 'Monitor under cursor', 'Primary monitor'];
export const VIDEO_FORMAT_OPTIONS = [
  'MP4 · H.264',
  'MP4 · H.265 (HEVC)',
  'WebM · VP9',
  'MKV · H.264',
];
export const RECORDING_FRAME_RATE_OPTIONS = ['24', '30', '60'];
export const QUALITY_OPTIONS = ['High (recommended)', 'Balanced', 'Small file', 'Lossless'];
export const GIF_FRAME_RATE_OPTIONS = ['10', '15', '20', '30'];
export const GIF_WIDTH_OPTIONS = ['Original size', '1280 px wide', '800 px wide', '480 px wide'];
export const MAX_SCROLL_OPTIONS = ['10', '20', '30', '50', 'Unlimited'];
export const RETRY_OPTIONS = ["Don't retry", 'Once', '3 times', '5 times'];
export const TIMEOUT_OPTIONS = ['30 s', '60 s', '120 s'];
export const HISTORY_OPTIONS = ['Forever', '1 year', '90 days', '30 days'];
export const PROXY_OPTIONS = ['Use system proxy', 'No proxy', 'Custom proxy…'];
export const MICROPHONE_OPTIONS = ['Default microphone'];
export const BUNDLED_FFMPEG_PATH = '%LOCALAPPDATA%\\Programs\\Snapline\\ffmpeg.exe';

export const NAME_TOKENS = [
  { token: '{app}', hint: 'Process name' },
  { token: '{window}', hint: 'Window title' },
  { token: '{yyyy}', hint: 'Year' },
  { token: '{MM}', hint: 'Month' },
  { token: '{dd}', hint: 'Day' },
  { token: '{HH}', hint: 'Hour' },
  { token: '{mm}', hint: 'Minute' },
  { token: '{ss}', hint: 'Second' },
  { token: '{rand}', hint: 'Random characters' },
  { token: '{n}', hint: 'Counter' },
  { token: '{w}', hint: 'Width' },
  { token: '{h}', hint: 'Height' },
];

const HOTKEY_ACTIONS = {
  captureRegion: 'capture_region',
  captureWindow: 'capture_window',
  captureFullScreen: 'capture_screen',
  scrollingCapture: 'capture_scrolling',
  lastRegion: 'capture_last_region',
  recordVideo: 'record_video',
  recordGif: 'record_gif',
  textCapture: 'tool_ocr',
  colourPicker: 'tool_colour',
  screenRuler: 'tool_ruler',
  uploadClipboard: 'upload_clipboard',
} as const;

const GENERAL_TEXT: ReadonlySet<string> = new Set(['saveFolder', 'namePattern']);
const CAPTURE_TEXT: ReadonlySet<string> = new Set(['microphoneDevice', 'ffmpegPath']);
const UPLOADS_TEXT: ReadonlySet<string> = new Set(['customProxy']);

function choose(value: string, labels: Record<string, string>, fallback: string): string {
  return labels[value] ?? fallback;
}

function imageFormatLabel(value: string): string {
  return choose(value, { png: 'PNG', jpeg: 'JPEG', webp: 'WebP' }, 'PNG');
}

export function imageFormatValue(label: string): string {
  return choose(label, { PNG: 'png', JPEG: 'jpeg', WebP: 'webp' }, 'png');
}

function linkFormatLabel(value: string): string {
  return choose(
    value,
    {
      direct: 'Direct link',
      markdown: 'Markdown',
      html: 'HTML <img>',
      bbcode: 'BBCode',
      page: 'Page link',
    },
    'Direct link',
  );
}

function linkFormatValue(label: string): string {
  return choose(
    label,
    {
      'Direct link': 'direct',
      Markdown: 'markdown',
      'HTML <img>': 'html',
      BBCode: 'bbcode',
      'Page link': 'page',
    },
    'direct',
  );
}

function subfolderLabel(value: string): string {
  return choose(
    value,
    {
      by_month: SUBFOLDER_OPTIONS[0],
      by_day: SUBFOLDER_OPTIONS[1],
      by_app: SUBFOLDER_OPTIONS[2],
      none: SUBFOLDER_OPTIONS[3],
    },
    SUBFOLDER_OPTIONS[0],
  );
}

function subfolderValue(label: string): string {
  return choose(
    label,
    {
      [SUBFOLDER_OPTIONS[0]]: 'by_month',
      [SUBFOLDER_OPTIONS[1]]: 'by_day',
      [SUBFOLDER_OPTIONS[2]]: 'by_app',
      [SUBFOLDER_OPTIONS[3]]: 'none',
    },
    'by_month',
  );
}

function captureDelayLabel(value: string): string {
  return choose(value, { off: 'Off', '1s': '1 s', '3s': '3 s', '5s': '5 s' }, 'Off');
}

function captureDelayValue(label: string): string {
  return choose(label, { Off: 'off', '1 s': '1s', '3 s': '3s', '5 s': '5s' }, 'off');
}

function zoomLabel(value: string): string {
  return choose(value, { '2x': '2×', '4x': '4×', '8x': '8×' }, '4×');
}

function zoomValue(label: string): string {
  return choose(label, { '2×': '2x', '4×': '4x', '8×': '8x' }, '4x');
}

function monitorLabel(value: string): string {
  return choose(
    value,
    {
      all_monitors: 'All monitors',
      monitor_under_cursor: 'Monitor under cursor',
      primary_monitor: 'Primary monitor',
    },
    'Monitor under cursor',
  );
}

function monitorValue(label: string): string {
  return choose(
    label,
    {
      'All monitors': 'all_monitors',
      'Monitor under cursor': 'monitor_under_cursor',
      'Primary monitor': 'primary_monitor',
    },
    'monitor_under_cursor',
  );
}

function videoFormatLabel(value: string): string {
  return choose(
    value,
    {
      mp4_h264: 'MP4 · H.264',
      mp4_h265_hevc: 'MP4 · H.265 (HEVC)',
      webm_vp9: 'WebM · VP9',
      mkv_h264: 'MKV · H.264',
    },
    'MP4 · H.264',
  );
}

function videoFormatValue(label: string): string {
  return choose(
    label,
    {
      'MP4 · H.264': 'mp4_h264',
      'MP4 · H.265 (HEVC)': 'mp4_h265_hevc',
      'WebM · VP9': 'webm_vp9',
      'MKV · H.264': 'mkv_h264',
    },
    'mp4_h264',
  );
}

function frameRateLabel(value: string): string {
  return choose(value, { fps24: '24', fps30: '30', fps60: '60' }, '30');
}

function frameRateValue(label: string): string {
  return choose(label, { '24': 'fps24', '30': 'fps30', '60': 'fps60' }, 'fps30');
}

function qualityLabel(value: string): string {
  return choose(
    value,
    {
      high: 'High (recommended)',
      balanced: 'Balanced',
      small_file: 'Small file',
      lossless: 'Lossless',
    },
    'High (recommended)',
  );
}

function qualityValue(label: string): string {
  return choose(
    label,
    {
      'High (recommended)': 'high',
      Balanced: 'balanced',
      'Small file': 'small_file',
      Lossless: 'lossless',
    },
    'high',
  );
}

function gifFrameRateLabel(value: string): string {
  return choose(value, { fps10: '10', fps15: '15', fps20: '20', fps30: '30' }, '15');
}

function gifFrameRateValue(label: string): string {
  return choose(label, { '10': 'fps10', '15': 'fps15', '20': 'fps20', '30': 'fps30' }, 'fps15');
}

function gifWidthLabel(value: string): string {
  return choose(
    value,
    {
      original: 'Original size',
      px1280: '1280 px wide',
      px800: '800 px wide',
      px480: '480 px wide',
    },
    '800 px wide',
  );
}

function gifWidthValue(label: string): string {
  return choose(
    label,
    {
      'Original size': 'original',
      '1280 px wide': 'px1280',
      '800 px wide': 'px800',
      '480 px wide': 'px480',
    },
    'px800',
  );
}

function scrollLimitLabel(value: string): string {
  return choose(
    value,
    { ten: '10', twenty: '20', thirty: '30', fifty: '50', unlimited: 'Unlimited' },
    '30',
  );
}

function scrollLimitValue(label: string): string {
  return choose(
    label,
    { '10': 'ten', '20': 'twenty', '30': 'thirty', '50': 'fifty', Unlimited: 'unlimited' },
    'thirty',
  );
}

function retryLabel(value: string): string {
  return choose(
    value,
    { never: "Don't retry", once: 'Once', three_times: '3 times', five_times: '5 times' },
    '3 times',
  );
}

function retryValue(label: string): string {
  return choose(
    label,
    { "Don't retry": 'never', Once: 'once', '3 times': 'three_times', '5 times': 'five_times' },
    'three_times',
  );
}

function timeoutLabel(value: string): string {
  return choose(value, { '30s': '30 s', '60s': '60 s', '120s': '120 s' }, '60 s');
}

function timeoutValue(label: string): string {
  return choose(label, { '30 s': '30s', '60 s': '60s', '120 s': '120s' }, '60s');
}

function historyLabel(value: string): string {
  return choose(
    value,
    { forever: 'Forever', one_year: '1 year', days90: '90 days', days30: '30 days' },
    'Forever',
  );
}

function historyValue(label: string): string {
  return choose(
    label,
    { Forever: 'forever', '1 year': 'one_year', '90 days': 'days90', '30 days': 'days30' },
    'forever',
  );
}

function proxyLabel(value: string): string {
  return choose(
    value,
    { system: 'Use system proxy', none: 'No proxy', custom: 'Custom proxy…' },
    'Use system proxy',
  );
}

function proxyValue(label: string): string {
  return choose(
    label,
    { 'Use system proxy': 'system', 'No proxy': 'none', 'Custom proxy…': 'custom' },
    'system',
  );
}

function destinationLabel(value: string): string {
  return choose(value, { save_locally_only: 'Save locally only', none: 'None' }, value);
}

function destinationValue(label: string): string {
  return choose(label, { 'Save locally only': 'save_locally_only', None: 'none' }, label);
}

export function toView(dto: SettingsDto): Settings {
  const bindings = dto.hotkeys.bindings;
  const chord = (action: string, fallback: string): string => bindings[action] ?? fallback;
  return {
    general: {
      launchAtSignIn: dto.general.launchAtSignin,
      startMinimised: dto.general.startMinimised,
      notifications: dto.general.notifications,
      saveFolder: dto.general.saveFolder,
      subfolders: subfolderLabel(dto.general.subfolders),
      imageFormat: imageFormatLabel(dto.general.imageFormat),
      lossyQuality: dto.general.lossyQuality,
      namePattern: dto.general.namePattern,
      defaultWorkflow: dto.general.defaultWorkflow.map((step) => ({
        kind: step.kind,
        enabled: step.enabled,
      })),
      copiedLinkFormat: linkFormatLabel(dto.general.copiedLinkFormat),
    },
    capture: {
      includeCursor: dto.capture.includeCursor,
      captureDelay: captureDelayLabel(dto.capture.captureDelay),
      shutterSound: dto.capture.shutterSound,
      windowShadow: dto.capture.keepWindowShadow,
      freezeScreen: dto.capture.freezeScreen,
      magnifier: dto.capture.magnifier,
      magnifierZoom: zoomLabel(dto.capture.magnifierZoom),
      snapWindows: dto.capture.snapToWindows,
      showSize: dto.capture.showSizeAndPosition,
      crosshair: dto.capture.fullscreenCrosshair,
      monitorMode: monitorLabel(dto.capture.fullscreenCaptureUses),
      videoFormat: videoFormatLabel(dto.capture.videoFormat),
      frameRate: frameRateLabel(dto.capture.frameRate),
      quality: qualityLabel(dto.capture.videoQuality),
      systemAudio: dto.capture.recordSystemAudio,
      microphone: dto.capture.recordMicrophone,
      microphoneDevice: dto.capture.microphoneDevice,
      recordingBorder: dto.capture.recordingBorder,
      ffmpegPath: dto.capture.ffmpegPath ?? '',
      gifFrameRate: gifFrameRateLabel(dto.capture.gifFrameRate),
      gifWidth: gifWidthLabel(dto.capture.gifMaxWidth),
      optimisePalette: dto.capture.gifOptimisePalette,
      scrollDelay: dto.capture.scrollDelayMs,
      maxScrolls: scrollLimitLabel(dto.capture.scrollLimit),
      removeStickyHeaders: dto.capture.removeStickyHeaders,
    },
    hotkeys: {
      captureRegion: chord(HOTKEY_ACTIONS.captureRegion, 'PrtSc'),
      captureWindow: chord(HOTKEY_ACTIONS.captureWindow, 'Alt+PrtSc'),
      captureFullScreen: chord(HOTKEY_ACTIONS.captureFullScreen, 'Ctrl+PrtSc'),
      scrollingCapture: chord(HOTKEY_ACTIONS.scrollingCapture, 'Ctrl+Alt+S'),
      lastRegion: chord(HOTKEY_ACTIONS.lastRegion, 'Ctrl+Shift+R'),
      recordVideo: chord(HOTKEY_ACTIONS.recordVideo, 'Shift+PrtSc'),
      recordGif: chord(HOTKEY_ACTIONS.recordGif, 'Ctrl+Shift+G'),
      textCapture: chord(HOTKEY_ACTIONS.textCapture, 'Ctrl+Shift+T'),
      colourPicker: chord(HOTKEY_ACTIONS.colourPicker, 'Ctrl+Shift+C'),
      screenRuler: chord(HOTKEY_ACTIONS.screenRuler, 'Ctrl+Shift+M'),
      uploadClipboard: chord(HOTKEY_ACTIONS.uploadClipboard, 'Ctrl+Shift+V'),
      takeOverPrtSc: dto.hotkeys.takeOverPrtsc,
      warnClashes: dto.hotkeys.warnAboutClashes,
    },
    uploads: {
      images: destinationLabel(dto.uploads.images),
      videos: destinationLabel(dto.uploads.videosAndGifs),
      text: destinationLabel(dto.uploads.textAndCode),
      otherFiles: destinationLabel(dto.uploads.otherFiles),
      linkShortener: destinationLabel(dto.uploads.linkShortener),
      removeMetadata: dto.uploads.stripExif,
      askClipboard: dto.uploads.askBeforeClipboardUpload,
      retry: retryLabel(dto.uploads.retry),
      timeout: timeoutLabel(dto.uploads.timeout),
      keepHistory: historyLabel(dto.uploads.keepHistory),
      proxy: proxyLabel(dto.uploads.proxy),
      customProxy: dto.uploads.proxyUrl ?? '',
    },
  };
}

export function defaultSettings(): Settings {
  return {
    general: {
      launchAtSignIn: true,
      startMinimised: true,
      notifications: true,
      saveFolder: '%USERPROFILE%\\Pictures\\Snapline',
      subfolders: SUBFOLDER_OPTIONS[0],
      imageFormat: 'PNG',
      lossyQuality: 90,
      namePattern: '{app}_{yyyy}-{MM}-{dd}_{HH}-{mm}-{ss}',
      defaultWorkflow: [
        { kind: 'copy_image', enabled: true },
        { kind: 'annotate', enabled: false },
        { kind: 'save', enabled: true },
        { kind: 'upload', enabled: true },
        { kind: 'copy_link', enabled: true },
        { kind: 'play_sound', enabled: true },
      ],
      copiedLinkFormat: 'Direct link',
    },
    capture: {
      includeCursor: false,
      captureDelay: 'Off',
      shutterSound: true,
      windowShadow: true,
      freezeScreen: true,
      magnifier: true,
      magnifierZoom: '4×',
      snapWindows: true,
      showSize: true,
      crosshair: false,
      monitorMode: 'Monitor under cursor',
      videoFormat: 'MP4 · H.264',
      frameRate: '30',
      quality: 'High (recommended)',
      systemAudio: true,
      microphone: false,
      microphoneDevice: 'Default microphone',
      recordingBorder: true,
      ffmpegPath: BUNDLED_FFMPEG_PATH,
      gifFrameRate: '15',
      gifWidth: '800 px wide',
      optimisePalette: true,
      scrollDelay: 300,
      maxScrolls: '30',
      removeStickyHeaders: true,
    },
    hotkeys: {
      captureRegion: 'PrtSc',
      captureWindow: 'Alt+PrtSc',
      captureFullScreen: 'Ctrl+PrtSc',
      scrollingCapture: 'Ctrl+Alt+S',
      lastRegion: 'Ctrl+Shift+R',
      recordVideo: 'Shift+PrtSc',
      recordGif: 'Ctrl+Shift+G',
      textCapture: 'Ctrl+Shift+T',
      colourPicker: 'Ctrl+Shift+C',
      screenRuler: 'Ctrl+Shift+M',
      uploadClipboard: 'Ctrl+Shift+V',
      takeOverPrtSc: true,
      warnClashes: true,
    },
    uploads: {
      images: 'Save locally only',
      videos: 'Save locally only',
      text: 'Save locally only',
      otherFiles: 'Save locally only',
      linkShortener: 'None',
      removeMetadata: true,
      askClipboard: true,
      retry: '3 times',
      timeout: '60 s',
      keepHistory: 'Forever',
      proxy: 'Use system proxy',
      customProxy: '',
    },
  };
}

export const settings = writable<Settings>(defaultSettings());
export const settingsError = writable<string | null>(null);

type Patch = Record<string, unknown>;

let pending: Patch = {};
let timer: ReturnType<typeof setTimeout> | undefined;
let generation = 0;

function isObject(value: unknown): value is Patch {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function mergePatch(target: Patch, patch: Patch): void {
  for (const [key, value] of Object.entries(patch)) {
    const existing = target[key];
    if (isObject(existing) && isObject(value)) {
      mergePatch(existing, value);
    } else {
      target[key] = value;
    }
  }
}

function errorMessage(error: unknown): string {
  const candidate = error as Partial<IpcError> | null;
  if (candidate && typeof candidate.message === 'string') {
    return candidate.message;
  }
  return 'The setting could not be saved.';
}

async function flush(): Promise<void> {
  if (timer !== undefined) {
    clearTimeout(timer);
    timer = undefined;
  }
  if (Object.keys(pending).length === 0) {
    return;
  }
  const patch = pending;
  pending = {};
  generation += 1;
  const mine = generation;
  try {
    const dto = await ipc.update(patch);
    if (mine === generation) {
      settings.set(toView(dto));
      settingsError.set(null);
    }
  } catch (error) {
    settingsError.set(errorMessage(error));
  }
}

function send(patch: Patch, debounce: boolean): void {
  mergePatch(pending, patch);
  if (debounce) {
    if (timer !== undefined) {
      clearTimeout(timer);
    }
    timer = setTimeout(() => {
      timer = undefined;
      void flush();
    }, 300);
  } else {
    void flush();
  }
}

function generalPatch<K extends keyof GeneralSettings>(
  key: K,
  value: GeneralSettings[K],
): Patch {
  switch (key) {
    case 'launchAtSignIn':
      return { launch_at_signin: value };
    case 'startMinimised':
      return { start_minimised: value };
    case 'notifications':
      return { notifications: value };
    case 'saveFolder':
      return { save_folder: value };
    case 'subfolders':
      return { subfolders: subfolderValue(String(value)) };
    case 'imageFormat':
      return { image_format: imageFormatValue(String(value)) };
    case 'lossyQuality':
      return { lossy_quality: value };
    case 'namePattern':
      return { name_pattern: value };
    case 'copiedLinkFormat':
      return { copied_link_format: linkFormatValue(String(value)) };
    default:
      return {};
  }
}

function capturePatch<K extends keyof CaptureSettings>(key: K, value: CaptureSettings[K]): Patch {
  switch (key) {
    case 'includeCursor':
      return { include_cursor: value };
    case 'captureDelay':
      return { capture_delay: captureDelayValue(String(value)) };
    case 'shutterSound':
      return { shutter_sound: value };
    case 'windowShadow':
      return { keep_window_shadow: value };
    case 'freezeScreen':
      return { freeze_screen: value };
    case 'magnifier':
      return { magnifier: value };
    case 'magnifierZoom':
      return { magnifier_zoom: zoomValue(String(value)) };
    case 'snapWindows':
      return { snap_to_windows: value };
    case 'showSize':
      return { show_size_and_position: value };
    case 'crosshair':
      return { fullscreen_crosshair: value };
    case 'monitorMode':
      return { fullscreen_capture_uses: monitorValue(String(value)) };
    case 'videoFormat':
      return { video_format: videoFormatValue(String(value)) };
    case 'frameRate':
      return { frame_rate: frameRateValue(String(value)) };
    case 'quality':
      return { video_quality: qualityValue(String(value)) };
    case 'systemAudio':
      return { record_system_audio: value };
    case 'microphone':
      return { record_microphone: value };
    case 'microphoneDevice':
      return { microphone_device: value };
    case 'recordingBorder':
      return { recording_border: value };
    case 'ffmpegPath':
      return { ffmpeg_path: value === '' ? null : value };
    case 'gifFrameRate':
      return { gif_frame_rate: gifFrameRateValue(String(value)) };
    case 'gifWidth':
      return { gif_max_width: gifWidthValue(String(value)) };
    case 'optimisePalette':
      return { gif_optimise_palette: value };
    case 'scrollDelay':
      return { scroll_delay_ms: value };
    case 'maxScrolls':
      return { scroll_limit: scrollLimitValue(String(value)) };
    case 'removeStickyHeaders':
      return { remove_sticky_headers: value };
    default:
      return {};
  }
}

function hotkeyPatch<K extends keyof HotkeySettings>(key: K, value: HotkeySettings[K]): Patch {
  switch (key) {
    case 'takeOverPrtSc':
      return { take_over_prtsc: value };
    case 'warnClashes':
      return { warn_about_clashes: value };
    case 'captureRegion':
      return { bindings: { [HOTKEY_ACTIONS.captureRegion]: value } };
    case 'captureWindow':
      return { bindings: { [HOTKEY_ACTIONS.captureWindow]: value } };
    case 'captureFullScreen':
      return { bindings: { [HOTKEY_ACTIONS.captureFullScreen]: value } };
    case 'scrollingCapture':
      return { bindings: { [HOTKEY_ACTIONS.scrollingCapture]: value } };
    case 'lastRegion':
      return { bindings: { [HOTKEY_ACTIONS.lastRegion]: value } };
    case 'recordVideo':
      return { bindings: { [HOTKEY_ACTIONS.recordVideo]: value } };
    case 'recordGif':
      return { bindings: { [HOTKEY_ACTIONS.recordGif]: value } };
    case 'textCapture':
      return { bindings: { [HOTKEY_ACTIONS.textCapture]: value } };
    case 'colourPicker':
      return { bindings: { [HOTKEY_ACTIONS.colourPicker]: value } };
    case 'screenRuler':
      return { bindings: { [HOTKEY_ACTIONS.screenRuler]: value } };
    case 'uploadClipboard':
      return { bindings: { [HOTKEY_ACTIONS.uploadClipboard]: value } };
    default:
      return {};
  }
}

function uploadsPatch<K extends keyof UploadsSettings>(
  key: K,
  value: UploadsSettings[K],
): Patch {
  switch (key) {
    case 'images':
      return { images: destinationValue(String(value)) };
    case 'videos':
      return { videos_and_gifs: destinationValue(String(value)) };
    case 'text':
      return { text_and_code: destinationValue(String(value)) };
    case 'otherFiles':
      return { other_files: destinationValue(String(value)) };
    case 'linkShortener':
      return { link_shortener: destinationValue(String(value)) };
    case 'removeMetadata':
      return { strip_exif: value };
    case 'askClipboard':
      return { ask_before_clipboard_upload: value };
    case 'retry':
      return { retry: retryValue(String(value)) };
    case 'timeout':
      return { timeout: timeoutValue(String(value)) };
    case 'keepHistory':
      return { keep_history: historyValue(String(value)) };
    case 'proxy':
      return { proxy: proxyValue(String(value)) };
    case 'customProxy':
      return { proxy_url: value === '' ? null : value };
    default:
      return {};
  }
}

export function updateGeneral<K extends keyof GeneralSettings>(
  key: K,
  value: GeneralSettings[K],
): void {
  settings.update((current) => ({
    ...current,
    general: { ...current.general, [key]: value },
  }));
  send({ general: generalPatch(key, value) }, GENERAL_TEXT.has(key));
}

export function updateWorkflow(steps: WorkflowStepSettings[]): void {
  const next = steps.map((step) => ({ kind: step.kind, enabled: step.enabled }));
  settings.update((current) => ({
    ...current,
    general: { ...current.general, defaultWorkflow: next },
  }));
  send({ general: { default_workflow: next } }, false);
}

export function updateCapture<K extends keyof CaptureSettings>(
  key: K,
  value: CaptureSettings[K],
): void {
  settings.update((current) => ({
    ...current,
    capture: { ...current.capture, [key]: value },
  }));
  send({ capture: capturePatch(key, value) }, CAPTURE_TEXT.has(key));
}

export function updateHotkeys<K extends keyof HotkeySettings>(
  key: K,
  value: HotkeySettings[K],
): void {
  settings.update((current) => ({
    ...current,
    hotkeys: { ...current.hotkeys, [key]: value },
  }));
  send({ hotkeys: hotkeyPatch(key, value) }, false);
}

export function updateUploads<K extends keyof UploadsSettings>(
  key: K,
  value: UploadsSettings[K],
): void {
  settings.update((current) => ({
    ...current,
    uploads: { ...current.uploads, [key]: value },
  }));
  send({ uploads: uploadsPatch(key, value) }, UPLOADS_TEXT.has(key));
}

export async function loadSettings(): Promise<void> {
  try {
    settings.set(toView(await ipc.get()));
    settingsError.set(null);
  } catch (error) {
    settingsError.set(errorMessage(error));
  }
}

export async function requestPreview(pattern: string, format: string): Promise<string> {
  try {
    return await ipc.previewName(pattern, format);
  } catch (error) {
    settingsError.set(errorMessage(error));
    return '';
  }
}

export async function pickFolder(current: string): Promise<string | null> {
  try {
    return await ipc.browseFolder(current === '' ? null : current);
  } catch (error) {
    settingsError.set(errorMessage(error));
    return null;
  }
}

export function resetSettings(): void {
  if (timer !== undefined) {
    clearTimeout(timer);
    timer = undefined;
  }
  pending = {};
  generation += 1;
  settings.set(defaultSettings());
  settingsError.set(null);
}
