import { afterEach, describe, expect, it } from 'vitest';
import { cleanup, render, screen } from '@testing-library/svelte';
import Icon from './Icon.svelte';
import { icons } from '../lib/icons';

afterEach(cleanup);

describe('Icon', () => {
  it('renders the path for its name', () => {
    const { container } = render(Icon, { props: { name: 'region', size: 20 } });
    const svg = container.querySelector('svg');
    expect(svg?.getAttribute('width')).toBe('20');
    expect(svg?.getAttribute('aria-hidden')).toBe('true');
    expect(container.querySelector('path')?.getAttribute('d')).toBe(icons.region);
    expect(screen.queryByRole('img')).toBeNull();
  });
});
