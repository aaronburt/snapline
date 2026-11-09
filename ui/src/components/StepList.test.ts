import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import StepList from './StepList.svelte';
import type { StepItem } from './types';

afterEach(cleanup);

const items: StepItem[] = [
  { id: 'copy', label: 'Copy image to clipboard', icon: 'copy', checked: true },
  { id: 'save', label: 'Save to folder', icon: 'folder', checked: true },
  { id: 'upload', label: 'Upload to default destination', icon: 'upload', checked: true },
];

describe('StepList', () => {
  it('renders numbered steps and reorder handles', () => {
    render(StepList, { props: { items } });
    expect(screen.getByText('Copy image to clipboard')).not.toBeNull();
    expect(screen.getByRole('button', { name: 'Reorder: Copy image to clipboard' })).not.toBeNull();
    expect(screen.getAllByRole('checkbox')).toHaveLength(3);
  });

  it('reorders with Alt+ArrowDown', async () => {
    const onchange = vi.fn();
    render(StepList, { props: { items, onchange } });
    await fireEvent.keyDown(
      screen.getByRole('button', { name: 'Reorder: Copy image to clipboard' }),
      { key: 'ArrowDown', altKey: true },
    );
    expect(onchange).toHaveBeenCalledOnce();
    expect(onchange.mock.calls[0][0].map((item: StepItem) => item.id)).toEqual([
      'save',
      'copy',
      'upload',
    ]);
  });
});
