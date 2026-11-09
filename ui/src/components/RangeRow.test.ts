import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import RangeRow from './RangeRow.svelte';

afterEach(cleanup);

describe('RangeRow', () => {
  it('shows the current value with its unit', async () => {
    const onchange = vi.fn();
    render(RangeRow, {
      props: {
        icon: 'gauge',
        title: 'Lossy quality',
        value: 90,
        min: 40,
        max: 100,
        unit: '%',
        onchange,
      },
    });
    expect(screen.getByText('90%')).not.toBeNull();
    const input = screen.getByRole('slider', { name: 'Lossy quality' });
    await fireEvent.input(input, { target: { value: '70' } });
    expect(onchange).toHaveBeenCalledWith(70);
    expect(screen.getByText('70%')).not.toBeNull();
  });
});
