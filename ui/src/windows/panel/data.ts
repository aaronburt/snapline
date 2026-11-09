import type { IconName } from '../../lib/icons';

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

export const panelRecents: PanelRecent[] = [
  {
    id: 1,
    name: 'Region capture, 2 minutes ago',
    time: '2 min ago',
    uploaded: true,
  },
  {
    id: 2,
    name: 'Screen recording, 18 minutes ago',
    time: '18 min ago',
    uploaded: true,
    duration: '0:42',
  },
  {
    id: 3,
    name: 'Window capture, 1 hour ago',
    time: '1 h ago',
    uploaded: false,
  },
];

export const panelLastLink = 'https://i.imgur.com/kq7Ra2P.png';
