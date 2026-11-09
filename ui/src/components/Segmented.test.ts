import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import Segmented from './Segmented.svelte';

afterEach(cleanup);

describe('Segmented', () => {
  it('selects one option at a time', async () => {
    const onchange = vi.fn();
    render(Segmented, {
      props: {
        label: 'Filter by type',
        options: ['All', 'Images', 'Videos'],
        value: 'All',
        onchange,
      },
    });
    expect(screen.getAllByRole('radio')).toHaveLength(3);
    await fireEvent.click(screen.getByRole('radio', { name: 'Videos' }));
    expect(onchange).toHaveBeenCalledWith('Videos');
    expect(screen.getByRole('radio', { name: 'Videos' }).getAttribute('aria-checked')).toBe(
      'true',
    );
  });
});
