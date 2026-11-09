import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import SegmentedRow from './SegmentedRow.svelte';

afterEach(cleanup);

describe('SegmentedRow', () => {
  it('renders a radio group labelled by the title', async () => {
    const onchange = vi.fn();
    render(SegmentedRow, {
      props: {
        icon: 'image',
        title: 'Image format',
        value: 'PNG',
        options: ['PNG', 'JPEG', 'WebP', 'AVIF'],
        onchange,
      },
    });
    expect(screen.getByRole('radiogroup', { name: 'Image format' })).not.toBeNull();
    expect(screen.getByRole('radio', { name: 'PNG' }).getAttribute('aria-checked')).toBe('true');
    await fireEvent.click(screen.getByRole('radio', { name: 'WebP' }));
    expect(onchange).toHaveBeenCalledWith('WebP');
    expect(screen.getByRole('radio', { name: 'WebP' }).getAttribute('aria-checked')).toBe('true');
  });
});
