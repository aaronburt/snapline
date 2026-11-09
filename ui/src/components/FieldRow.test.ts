import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import FieldRow from './FieldRow.svelte';

afterEach(cleanup);

describe('FieldRow', () => {
  it('labels the input and renders the secondary button', async () => {
    const onbutton = vi.fn();
    render(FieldRow, {
      props: {
        icon: 'folder',
        title: 'Save captures to',
        description: 'Captures are kept on this PC even after they are uploaded',
        value: '%USERPROFILE%\\Pictures\\Snapline',
        mono: true,
        button: 'Browse…',
        onbutton,
      },
    });
    const input = screen.getByRole('textbox', { name: 'Save captures to' });
    expect((input as HTMLInputElement).value).toBe('%USERPROFILE%\\Pictures\\Snapline');
    await fireEvent.click(screen.getByRole('button', { name: 'Browse…' }));
    expect(onbutton).toHaveBeenCalledOnce();
  });
});
