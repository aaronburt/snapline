import { afterEach, describe, expect, it } from 'vitest';
import { cleanup, render } from '@testing-library/svelte';
import Logo from './Logo.svelte';

afterEach(cleanup);

describe('Logo', () => {
  it('draws the accent square with the corner mark', () => {
    const { container } = render(Logo, { props: { size: 32 } });
    const rect = container.querySelector('rect');
    expect(rect?.getAttribute('fill')).toBe('var(--accent)');
    expect(rect?.getAttribute('rx')).toBe('8');
    expect(container.querySelector('circle')).not.toBeNull();
  });

  it('can omit the centre dot', () => {
    const { container } = render(Logo, { props: { size: 16, dot: false } });
    expect(container.querySelector('circle')).toBeNull();
  });
});
