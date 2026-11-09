import type { CaptureKindDto } from '../../../lib/ipc/bindings/CaptureKindDto';
import type { HistoryCaptureDto } from '../../../lib/ipc/bindings/HistoryCaptureDto';
import type { HistoryFilter } from '../../../lib/ipc/bindings/HistoryFilter';

export const historyFilters = ['All', 'Images', 'Videos', 'GIFs'];
export const captureCardWidth = 200;
export const captureCardGap = 12;

export const captureCount = '';

export function filterValue(label: string): HistoryFilter {
  if (label === 'Images') {
    return 'images';
  }
  if (label === 'Videos') {
    return 'videos';
  }
  if (label === 'GIFs') {
    return 'gifs';
  }
  return 'all';
}

export interface CaptureGroup {
  key: string;
  label: string;
  countLabel: string;
  items: HistoryCaptureDto[];
}

export function groupCaptures(items: HistoryCaptureDto[]): CaptureGroup[] {
  const order: string[] = [];
  const groups = new Map<string, HistoryCaptureDto[]>();
  for (const item of items) {
    const key = dayKey(item.createdAt);
    if (!groups.has(key)) {
      groups.set(key, []);
      order.push(key);
    }
    groups.get(key)?.push(item);
  }
  return order.map((key) => {
    const members = groups.get(key) ?? [];
    return {
      key,
      label: dayLabel(members[0]?.createdAt ?? Date.now()),
      countLabel: members.length === 1 ? '1 capture' : `${members.length} captures`,
      items: members,
    };
  });
}

export function dayKey(milliseconds: number): string {
  const date = new Date(milliseconds);
  return `${date.getFullYear()}-${date.getMonth()}-${date.getDate()}`;
}

export function dayLabel(milliseconds: number): string {
  const date = new Date(milliseconds);
  const today = new Date();
  const yesterday = new Date(today.getFullYear(), today.getMonth(), today.getDate() - 1);
  if (isSameDay(date, today)) {
    return 'Today';
  }
  if (isSameDay(date, yesterday)) {
    return 'Yesterday';
  }
  const showYear = date.getFullYear() !== today.getFullYear();
  const formatter = new Intl.DateTimeFormat('en-GB', {
    weekday: 'long',
    day: 'numeric',
    month: 'long',
    ...(showYear ? { year: 'numeric' } : {}),
  });
  return formatter.format(date);
}

export const formatDay = dayLabel;

export function formatTime(milliseconds: number): string {
  const date = new Date(milliseconds);
  return `${String(date.getHours()).padStart(2, '0')}:${String(date.getMinutes()).padStart(2, '0')}`;
}

export function formatCaptured(milliseconds: number): string {
  const date = new Date(milliseconds);
  const today = new Date();
  if (isSameDay(date, today)) {
    return `Today, ${formatTime(milliseconds)}`;
  }
  const formatter = new Intl.DateTimeFormat('en-GB', {
    day: 'numeric',
    month: 'short',
    ...(date.getFullYear() !== today.getFullYear() ? { year: 'numeric' } : {}),
  });
  return `${formatter.format(date)}, ${formatTime(milliseconds)}`;
}

export function formatDuration(milliseconds: number | null): string {
  if (milliseconds === null || milliseconds < 0) {
    return '0:00';
  }
  const total = Math.round(milliseconds / 1000);
  const minutes = Math.floor(total / 60);
  const seconds = total % 60;
  return `${minutes}:${String(seconds).padStart(2, '0')}`;
}

export function formatBytes(bytes: number): string {
  if (bytes < 1024) {
    return `${bytes} bytes`;
  }
  if (bytes < 1024 * 1024) {
    return `${Math.round(bytes / 1024)} KB`;
  }
  if (bytes < 1024 * 1024 * 1024) {
    return `${oneDecimal(bytes / (1024 * 1024))} MB`;
  }
  return `${oneDecimal(bytes / (1024 * 1024 * 1024))} GB`;
}

function oneDecimal(value: number): string {
  return new Intl.NumberFormat('en-GB', {
    minimumFractionDigits: 1,
    maximumFractionDigits: 1,
  }).format(value);
}

export function typeLabel(kind: CaptureKindDto, name: string): string {
  if (kind === 'gif') {
    return 'GIF animation';
  }
  const extension = fileExtension(name);
  if (kind === 'video') {
    return `${extension.toUpperCase()} video`;
  }
  const image =
    extension === 'jpg' ? 'JPEG' : extension === 'webp' ? 'WebP' : extension.toUpperCase();
  return `${image || 'Image'} image`;
}

export function typeTag(kind: CaptureKindDto, name: string): string {
  if (kind === 'gif') {
    return 'GIF';
  }
  return fileExtension(name).toUpperCase() || (kind === 'video' ? 'VIDEO' : 'IMAGE');
}

export function fileExtension(name: string): string {
  const dot = name.lastIndexOf('.');
  if (dot === -1 || dot === name.length - 1) {
    return '';
  }
  return name.slice(dot + 1).toLowerCase();
}

export type BadgeStatus = 'ok' | 'local' | 'failed' | 'uploading';

export function badgeStatus(capture: HistoryCaptureDto): BadgeStatus {
  const state = capture.upload?.state;
  if (state === 'done') {
    return 'ok';
  }
  if (state === 'failed') {
    return 'failed';
  }
  if (state === 'queued' || state === 'uploading') {
    return 'uploading';
  }
  return 'local';
}

export function timed(capture: HistoryCaptureDto): boolean {
  return capture.kind !== 'image';
}

export function isSameDay(left: Date, right: Date): boolean {
  return (
    left.getFullYear() === right.getFullYear() &&
    left.getMonth() === right.getMonth() &&
    left.getDate() === right.getDate()
  );
}
