export type CaptureType = 'image' | 'video' | 'gif';
export type CaptureStatus = 'up' | 'local' | 'fail';

export interface Capture {
  id: number;
  day: string;
  name: string;
  type: CaptureType;
  time: string;
  dim: string;
  size: string;
  status: CaptureStatus;
  app: string;
  url?: string;
  dest?: string;
  dur?: string;
  text?: string;
}

export type HistoryFilter = 'All' | 'Images' | 'Videos' | 'GIFs' | 'Uploaded' | 'Local only';

export const historyFilters: HistoryFilter[] = [
  'All',
  'Images',
  'Videos',
  'GIFs',
  'Uploaded',
  'Local only',
];

export const captures: Capture[] = [
  {
    id: 1,
    day: 'Today',
    name: 'Region_2026-11-08_14-31-07.png',
    type: 'image',
    time: '14:31',
    dim: '1280 × 720',
    size: '412 KB',
    status: 'up',
    app: 'chrome.exe',
    url: 'https://i.imgur.com/kq7Ra2P.png',
    dest: 'Imgur',
  },
  {
    id: 2,
    day: 'Today',
    name: 'Recording_2026-11-08_14-14-52.mp4',
    type: 'video',
    time: '14:14',
    dim: '1920 × 1080',
    size: '18.4 MB',
    status: 'up',
    app: 'Code.exe',
    url: 'https://files.example.com/r/8hQ2xT.mp4',
    dest: 'Amazon S3',
    dur: '0:42',
  },
  {
    id: 3,
    day: 'Today',
    name: 'Window_2026-11-08_13-40-11.png',
    type: 'image',
    time: '13:40',
    dim: '1102 × 764',
    size: '236 KB',
    status: 'local',
    app: 'explorer.exe',
  },
  {
    id: 4,
    day: 'Today',
    name: 'Scroll_2026-11-08_12-02-33.png',
    type: 'image',
    time: '12:02',
    dim: '1280 × 4820',
    size: '2.1 MB',
    status: 'up',
    app: 'firefox.exe',
    url: 'https://i.imgur.com/Vb31QeL.png',
    dest: 'Imgur',
  },
  {
    id: 5,
    day: 'Today',
    name: 'Region_2026-11-08_11-47-09.gif',
    type: 'gif',
    time: '11:47',
    dim: '800 × 450',
    size: '3.6 MB',
    status: 'fail',
    app: 'Discord.exe',
    dur: '0:08',
  },
  {
    id: 6,
    day: 'Today',
    name: 'Region_2026-11-08_10-15-40.png',
    type: 'image',
    time: '10:15',
    dim: '640 × 360',
    size: '98 KB',
    status: 'up',
    app: 'Figma.exe',
    url: 'https://i.imgur.com/p2mZr8a.png',
    dest: 'Imgur',
  },
  {
    id: 7,
    day: 'Yesterday',
    name: 'Screen_2026-11-07_18-22-05.png',
    type: 'image',
    time: '18:22',
    dim: '3840 × 1080',
    size: '1.4 MB',
    status: 'up',
    app: 'Desktop',
    url: 'https://i.imgur.com/Hc7Wn0d.png',
    dest: 'Imgur',
  },
  {
    id: 8,
    day: 'Yesterday',
    name: 'Region_2026-11-07_16-09-44.png',
    type: 'image',
    time: '16:09',
    dim: '512 × 512',
    size: '64 KB',
    status: 'up',
    app: 'chrome.exe',
    url: 'https://i.imgur.com/T4xa9Kp.png',
    dest: 'Imgur',
  },
  {
    id: 9,
    day: 'Yesterday',
    name: 'Recording_2026-11-07_15-30-12.mp4',
    type: 'video',
    time: '15:30',
    dim: '2560 × 1440',
    size: '52.3 MB',
    status: 'local',
    app: 'Code.exe',
    dur: '1:58',
  },
  {
    id: 10,
    day: 'Yesterday',
    name: 'Window_2026-11-07_09-58-01.png',
    type: 'image',
    time: '09:58',
    dim: '1600 × 900',
    size: '318 KB',
    status: 'up',
    app: 'Outlook.exe',
    url: 'https://i.imgur.com/Ld8sQw3.png',
    dest: 'Imgur',
  },
];

export const captureCount = '248';
export const diskSize = '1.2 GB';
export const uploadedThisWeek = '31';
export const defaultDestination = 'Imgur';

export function filterCaptures(
  items: Capture[],
  filter: HistoryFilter,
  query: string,
): Capture[] {
  const term = query.trim().toLowerCase();
  return items.filter((item) => {
    if (term && !`${item.name} ${item.app} ${item.text ?? ''}`.toLowerCase().includes(term)) {
      return false;
    }
    if (filter === 'All') {
      return true;
    }
    if (filter === 'Images') {
      return item.type === 'image';
    }
    if (filter === 'Videos') {
      return item.type === 'video';
    }
    if (filter === 'GIFs') {
      return item.type === 'gif';
    }
    if (filter === 'Uploaded') {
      return item.status === 'up';
    }
    return item.status === 'local';
  });
}

export interface CaptureGroup {
  label: string;
  count: string;
  items: Capture[];
}

export function groupCaptures(items: Capture[]): CaptureGroup[] {
  const order: string[] = [];
  const groups = new Map<string, Capture[]>();
  for (const item of items) {
    if (!groups.has(item.day)) {
      groups.set(item.day, []);
      order.push(item.day);
    }
    groups.get(item.day)?.push(item);
  }
  return order.map((day) => {
    const members = groups.get(day) ?? [];
    return {
      label: day,
      count: members.length === 1 ? '1 capture' : `${members.length} captures`,
      items: members,
    };
  });
}

export function typeLabel(type: CaptureType): string {
  if (type === 'video') {
    return 'MP4 video';
  }
  if (type === 'gif') {
    return 'GIF animation';
  }
  return 'PNG image';
}

export function typeTag(type: CaptureType): string {
  return type === 'gif' ? 'GIF' : 'MP4';
}

export function badgeStatus(status: CaptureStatus): 'ok' | 'local' | 'failed' {
  if (status === 'up') {
    return 'ok';
  }
  if (status === 'fail') {
    return 'failed';
  }
  return 'local';
}

export function moveSelection(items: Capture[], selectedId: number | null, step: number): number | null {
  if (items.length === 0) {
    return null;
  }
  const index = items.findIndex((item) => item.id === selectedId);
  const start = index === -1 ? 0 : index;
  const next = Math.min(items.length - 1, Math.max(0, start + step));
  return items[next]?.id ?? null;
}
