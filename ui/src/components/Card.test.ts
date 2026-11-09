import { afterEach, describe, expect, it } from 'vitest';
import { cleanup, render, screen } from '@testing-library/svelte';
import { createRawSnippet } from 'svelte';
import Card from './Card.svelte';

afterEach(cleanup);

describe('Card', () => {
  it('renders its content', () => {
    const children = createRawSnippet(() => ({ render: () => '<span>Capture details</span>' }));
    render(Card, { props: { children } });
    expect(screen.getByText('Capture details')).not.toBeNull();
  });
});
