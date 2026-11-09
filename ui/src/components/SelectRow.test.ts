import { afterEach, describe, expect, it } from 'vitest';
import { cleanup, render, screen } from '@testing-library/svelte';
import SelectRow from './SelectRow.svelte';

afterEach(cleanup);

describe('SelectRow', () => {
  it('labels the select with the row title', () => {
    render(SelectRow, {
      props: {
        icon: 'image',
        title: 'Images',
        description: '',
        value: 'Imgur',
        options: ['Imgur', 'Amazon S3', 'Save locally only'],
      },
    });
    const select = screen.getByRole('combobox', { name: 'Images' });
    expect((select as HTMLSelectElement).value).toBe('Imgur');
    expect(screen.getByRole('option', { name: 'Amazon S3' })).not.toBeNull();
  });
});
