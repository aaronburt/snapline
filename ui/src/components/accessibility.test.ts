import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render } from '@testing-library/svelte';
import { createRawSnippet } from 'svelte';
import { expectAccessible } from '../test/axe';
import Badge from './Badge.svelte';
import Button from './Button.svelte';
import CaptionButtons from './CaptionButtons.svelte';
import Card from './Card.svelte';
import EmptyState from './EmptyState.svelte';
import FieldRow from './FieldRow.svelte';
import HotkeyField from './HotkeyField.svelte';
import Icon from './Icon.svelte';
import IconButton from './IconButton.svelte';
import Logo from './Logo.svelte';
import NavItem from './NavItem.svelte';
import Popover from './Popover.svelte';
import RangeRow from './RangeRow.svelte';
import Segmented from './Segmented.svelte';
import SegmentedRow from './SegmentedRow.svelte';
import SelectRow from './SelectRow.svelte';
import StepList from './StepList.svelte';
import Thumb from './Thumb.svelte';
import TitleBar from './TitleBar.svelte';
import ToggleRow from './ToggleRow.svelte';
import type { StepItem } from './types';

const appWindow = vi.hoisted(() => ({
  minimize: vi.fn(),
  toggleMaximize: vi.fn(),
  close: vi.fn(),
  isMaximized: vi.fn().mockResolvedValue(false),
  onResized: vi.fn().mockResolvedValue(() => undefined),
}));

vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => appWindow,
}));

const label = createRawSnippet(() => ({ render: () => '<span>Save</span>' }));

const steps: StepItem[] = [
  { id: 'copy', label: 'Copy image to clipboard', icon: 'copy', checked: true },
  { id: 'save', label: 'Save to folder', icon: 'folder', checked: false },
];

afterEach(cleanup);

describe('component accessibility', () => {
  it('Badge', async () => {
    const { container } = render(Badge, { props: { status: 'ok', long: true } });
    await expectAccessible(container);
  });

  it('Button', async () => {
    const { container } = render(Button, {
      props: { variant: 'accent', height: 34, children: label },
    });
    await expectAccessible(container);
  });

  it('CaptionButtons', async () => {
    const { container } = render(CaptionButtons);
    await expectAccessible(container);
  });

  it('Card', async () => {
    const { container } = render(Card, { props: { children: label } });
    await expectAccessible(container);
  });

  it('EmptyState', async () => {
    const { container } = render(EmptyState, {
      props: { title: 'No captures yet', text: 'Press PrtSc to capture a region.' },
    });
    await expectAccessible(container);
  });

  it('FieldRow', async () => {
    const { container } = render(FieldRow, {
      props: {
        icon: 'folder',
        title: 'Save captures to',
        description: 'Captures are kept on this PC even after they are uploaded',
        value: 'C:\\Captures',
        button: 'Browse…',
      },
    });
    await expectAccessible(container);
  });

  it('HotkeyField', async () => {
    const { container } = render(HotkeyField, {
      props: { label: 'Capture region', value: 'PrtSc' },
    });
    await expectAccessible(container);
  });

  it('Icon', async () => {
    const { container } = render(Icon, { props: { name: 'search' } });
    await expectAccessible(container);
  });

  it('IconButton', async () => {
    const { container } = render(IconButton, { props: { label: 'Delete', icon: 'trash' } });
    await expectAccessible(container);
  });

  it('Logo', async () => {
    const { container } = render(Logo, { props: { size: 32 } });
    await expectAccessible(container);
  });

  it('NavItem', async () => {
    const { container } = render(NavItem, {
      props: { label: 'History', icon: 'history', count: '12', href: '#/history', active: true },
    });
    await expectAccessible(container);
  });

  it('Popover', async () => {
    const { container } = render(Popover, {
      props: { open: true, label: 'QR code', children: label },
    });
    await expectAccessible(container);
  });

  it('RangeRow', async () => {
    const { container } = render(RangeRow, {
      props: {
        icon: 'gauge',
        title: 'Lossy quality',
        description: 'Used for JPEG and WebP',
        value: 90,
        unit: '%',
      },
    });
    await expectAccessible(container);
  });

  it('Segmented', async () => {
    const { container } = render(Segmented, {
      props: {
        options: ['All', 'Images', 'Videos'],
        value: 'All',
        label: 'Filter by type',
      },
    });
    await expectAccessible(container);
  });

  it('SegmentedRow', async () => {
    const { container } = render(SegmentedRow, {
      props: {
        icon: 'image',
        title: 'Image format',
        description: 'PNG is lossless; the others are smaller',
        options: ['PNG', 'JPEG', 'WebP'],
        value: 'PNG',
      },
    });
    await expectAccessible(container);
  });

  it('SelectRow', async () => {
    const { container } = render(SelectRow, {
      props: {
        icon: 'tree',
        title: 'Subfolders',
        description: 'Organise saved files automatically',
        options: ['No subfolders', 'By month'],
        value: 'No subfolders',
      },
    });
    await expectAccessible(container);
  });

  it('StepList', async () => {
    const { container } = render(StepList, { props: { items: steps } });
    await expectAccessible(container);
  });

  it('Thumb', async () => {
    const { container } = render(Thumb, {
      props: { id: 1, alt: 'capture.png', duration: '0:42', tag: 'MP4' },
    });
    await expectAccessible(container);
  });

  it('TitleBar', async () => {
    const { container } = render(TitleBar);
    await expectAccessible(container);
  });

  it('ToggleRow', async () => {
    const { container } = render(ToggleRow, {
      props: {
        icon: 'power',
        title: 'Launch at sign-in',
        description: 'Start Snapline in the tray when you sign in to Windows',
        checked: true,
      },
    });
    await expectAccessible(container);
  });
});

describe('component labels', () => {
  it('names icon-only controls', async () => {
    render(IconButton, { props: { label: 'Copy image', icon: 'copy' } });
    await expectAccessible(document.body);
  });
});
