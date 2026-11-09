import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import IconButton from './IconButton.svelte';

afterEach(cleanup);

describe('IconButton', () => {
  it('exposes its label and pressed state', async () => {
    const onclick = vi.fn();
    render(IconButton, {
      props: { label: 'Grid view', icon: 'grid', pressed: true, onclick },
    });
    const button = screen.getByRole('button', { name: 'Grid view' });
    expect(button.getAttribute('aria-pressed')).toBe('true');
    await fireEvent.click(button);
    expect(onclick).toHaveBeenCalledOnce();
  });
});
