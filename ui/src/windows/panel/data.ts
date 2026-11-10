import type { IconName } from '../../lib/icons';
import type { RecentCaptureDto } from '../../lib/ipc/bindings/RecentCaptureDto';

export interface PanelCapture {
  id: string;
  label: string;
  chord: string;
  icon: IconName;
  action: string;
  accent?: boolean;
}

export const panelCaptures: PanelCapture[] = [
  {
    id: 'region',
    label: 'Region',
    chord: 'PrtSc',
    icon: 'region',
    action: 'capture_region',
    accent: true,
  },
  {
    id: 'window',
    label: 'Window',
    chord: 'Alt+PrtSc',
    icon: 'window',
    action: 'capture_window',
  },
  {
    id: 'screen',
    label: 'Screen',
    chord: 'Ctrl+PrtSc',
    icon: 'screen',
    action: 'capture_screen',
  },
  {
    id: 'scrolling',
    label: 'Scrolling',
    chord: 'Ctrl+Alt+S',
    icon: 'scroll',
    action: 'capture_scrolling',
  },
];

export interface PanelRecent {
  id: number;
  name: string;
  time: string;
  uploaded: boolean;
  duration?: string;
}

export function toPanelRecent(capture: RecentCaptureDto, now: number = Date.now()): PanelRecent {
  return {
    id: capture.id,
    name: capture.name,
    time: relativeTime(capture.createdAt, now),
    uploaded: capture.status === 'uploaded',
    duration:
      capture.kind === 'image' || capture.durationMs === null
        ? undefined
        : formatDuration(capture.durationMs),
  };
}

export function relativeTime(milliseconds: number, now: number = Date.now()): string {
  const date = new Date(milliseconds);
  const current = new Date(now);
  if (isSameDay(date, current)) {
    const minutes = Math.floor((now - milliseconds) / 60_000);
    if (minutes < 60) {
      return `${Math.max(1, minutes)} min ago`;
    }
    return `${Math.floor(minutes / 60)} h ago`;
  }
  const yesterday = new Date(current.getFullYear(), current.getMonth(), current.getDate() - 1);
  if (isSameDay(date, yesterday)) {
    return 'Yesterday';
  }
  return new Intl.DateTimeFormat('en-GB', { day: 'numeric', month: 'short' }).format(date);
}

export function formatDuration(milliseconds: number): string {
  const total = Math.round(milliseconds / 1000);
  const minutes = Math.floor(total / 60);
  const seconds = total % 60;
  return `${minutes}:${String(seconds).padStart(2, '0')}`;
}

export function isSameDay(left: Date, right: Date): boolean {
  return (
    left.getFullYear() === right.getFullYear() &&
    left.getMonth() === right.getMonth() &&
    left.getDate() === right.getDate()
  );
}
