import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { createRawSnippet } from 'svelte';
import Button from './Button.svelte';

afterEach(cleanup);

describe('Button', () => {
  it('renders its label and calls onclick', async () => {
    const onclick = vi.fn();
    const children = createRawSnippet(() => ({ render: () => '<span>Save</span>' }));
    render(Button, { props: { variant: 'accent', height: 34, children, onclick } });
    const button = screen.getByRole('button', { name: 'Save' });
    expect(button.className).toContain('accent');
    expect(button.style.height).toBe('34px');
    await fireEvent.click(button);
    expect(onclick).toHaveBeenCalledOnce();
  });
});
