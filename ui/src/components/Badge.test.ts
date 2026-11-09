import { afterEach, describe, expect, it } from 'vitest';
import { cleanup, render, screen } from '@testing-library/svelte';
import Badge from './Badge.svelte';

afterEach(cleanup);

describe('Badge', () => {
  it('renders the short label for each status', () => {
    render(Badge, { props: { status: 'ok' } });
    expect(screen.getByText('Link')).not.toBeNull();
    cleanup();
    render(Badge, { props: { status: 'local' } });
    expect(screen.getByText('Local')).not.toBeNull();
    cleanup();
    render(Badge, { props: { status: 'failed' } });
    expect(screen.getByText('Failed')).not.toBeNull();
    cleanup();
    render(Badge, { props: { status: 'uploading' } });
    expect(screen.getByText('Uploading')).not.toBeNull();
  });

  it('renders the long label when asked', () => {
    render(Badge, { props: { status: 'failed', long: true } });
    expect(screen.getByText('Upload failed')).not.toBeNull();
  });
});
