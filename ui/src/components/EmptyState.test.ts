import { afterEach, describe, expect, it } from 'vitest';
import { cleanup, render, screen } from '@testing-library/svelte';
import EmptyState from './EmptyState.svelte';

afterEach(cleanup);

describe('EmptyState', () => {
  it('renders the title and the line', () => {
    render(EmptyState, {
      props: { title: 'No captures yet', text: 'Press PrtSc to capture a region.' },
    });
    expect(screen.getByText('No captures yet')).not.toBeNull();
    expect(screen.getByText('Press PrtSc to capture a region.')).not.toBeNull();
  });
});
