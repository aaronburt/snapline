import { afterEach, describe, expect, it } from 'vitest';
import { cleanup, render, screen } from '@testing-library/svelte';
import Thumb from './Thumb.svelte';

afterEach(cleanup);

describe('Thumb', () => {
  it('loads the thumbnail from the thumb scheme', () => {
    render(Thumb, { props: { id: 42, alt: 'Region capture', tag: 'MP4', duration: '0:42' } });
    const image = screen.getByRole('img', { name: 'Region capture' }) as HTMLImageElement;
    expect(image.src).toBe('http://thumb.localhost/42');
    expect(image.loading).toBe('lazy');
    expect(image.decoding).toBe('async');
    expect(screen.getByText('MP4 · 0:42')).not.toBeNull();
  });
});
