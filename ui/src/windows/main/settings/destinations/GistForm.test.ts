import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import type { DestinationView } from '../../../../lib/ipc/upload';
import GistForm from './GistForm.svelte';

function view(overrides: Partial<DestinationView> = {}): DestinationView {
  return {
    id: 'gist',
    kind: 'gist',
    name: 'GitHub Gist',
    detail: 'Text and code snippets',
    status: 'not_set_up',
    action: 'set_up',
    configured: false,
    clientIdSaved: false,
    endpoint: null,
    region: null,
    bucket: null,
    pathPrefix: null,
    publicUrlPattern: null,
    aclPublicRead: false,
    accessKeyIdSaved: false,
    secretAccessKeySaved: false,
    method: null,
    requestUrl: null,
    body: null,
    fileFormField: null,
    link: null,
    thumbnail: null,
    deletion: null,
    errorMessage: null,
    headers: [],
    importedFromSxcu: false,
    warnings: [],
    ...overrides,
  };
}

function form(target = view()): {
  onsave: ReturnType<typeof vi.fn>;
  ontest: ReturnType<typeof vi.fn>;
  onremove: ReturnType<typeof vi.fn>;
} {
  const onsave = vi.fn().mockResolvedValue(view({ configured: true, status: 'configured' }));
  const ontest = vi.fn().mockResolvedValue({
    status: 'succeeded',
    link: 'https://gist.github.com/a',
    message: null,
  });
  const onremove = vi.fn().mockResolvedValue(undefined);
  render(GistForm, { props: { view: target, onsave, ontest, onremove } });
  return { onsave, ontest, onremove };
}

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe('GistForm', () => {
  it('renders the Token, Public, Test upload, Save and Remove destination controls', () => {
    form();
    const token = screen.getByLabelText('Token') as HTMLInputElement;
    expect(token.type).toBe('password');
    expect(token.placeholder).toBe('');
    expect(screen.getByRole('checkbox')).not.toBeNull();
    expect(screen.getByText('Public')).not.toBeNull();
    expect(screen.getByRole('button', { name: 'Test upload' })).not.toBeNull();
    expect(screen.getByRole('button', { name: 'Save' })).not.toBeNull();
    expect(screen.getByRole('button', { name: 'Remove destination' })).not.toBeNull();
    expect(screen.getByRole('status').textContent?.trim()).toBe('Not tested yet');
  });

  it('shows and hides the token with an aria-pressed button', async () => {
    form();
    const token = screen.getByLabelText('Token') as HTMLInputElement;
    const toggle = screen.getByRole('button', { name: 'Show Token' });
    expect(toggle.getAttribute('aria-pressed')).toBe('false');
    await fireEvent.click(toggle);
    expect(token.type).toBe('text');
    expect(
      screen.getByRole('button', { name: 'Hide Token' }).getAttribute('aria-pressed'),
    ).toBe('true');
    await fireEvent.click(screen.getByRole('button', { name: 'Hide Token' }));
    expect(token.type).toBe('password');
  });

  it('saves the token and the public toggle, then clears the token', async () => {
    const { onsave } = form();
    const token = screen.getByLabelText('Token') as HTMLInputElement;
    await fireEvent.input(token, { target: { value: 'ghp_example' } });
    await fireEvent.click(screen.getByRole('checkbox'));
    await fireEvent.click(screen.getByRole('button', { name: 'Save' }));
    await waitFor(() =>
      expect(onsave).toHaveBeenCalledWith({
        id: 'gist',
        kind: 'gist',
        clientId: 'ghp_example',
        aclPublicRead: true,
        headers: [],
      }),
    );
    expect(token.value).toBe('');
    expect(token.placeholder).toBe('Saved');
  });

  it('requires a token when none is saved', async () => {
    const { onsave } = form();
    await fireEvent.click(screen.getByRole('button', { name: 'Save' }));
    expect(onsave).not.toHaveBeenCalled();
    expect(await screen.findByText('Enter the GitHub token.')).not.toBeNull();
  });

  it('keeps a saved token without asking for it again', async () => {
    const { onsave } = form(view({ configured: true, status: 'configured' }));
    const token = screen.getByLabelText('Token') as HTMLInputElement;
    expect(token.placeholder).toBe('Saved');
    await fireEvent.click(screen.getByRole('button', { name: 'Save' }));
    await waitFor(() =>
      expect(onsave).toHaveBeenCalledWith({
        id: 'gist',
        kind: 'gist',
        clientId: null,
        aclPublicRead: false,
        headers: [],
      }),
    );
  });

  it('runs a test upload through the service and shows the result', async () => {
    const { ontest } = form(view({ configured: true, status: 'configured' }));
    await fireEvent.click(screen.getByRole('button', { name: 'Test upload' }));
    await waitFor(() => expect(ontest).toHaveBeenCalledWith('gist'));
    expect((await screen.findByRole('status')).textContent?.trim()).toBe(
      'Test upload succeeded · link copied',
    );
  });

  it('shows a failed test upload message', async () => {
    const target = view({ configured: true, status: 'configured' });
    const onsave = vi.fn().mockResolvedValue(target);
    const ontest = vi.fn().mockResolvedValue({
      status: 'failed',
      link: null,
      message: 'Bad credentials',
    });
    render(GistForm, { props: { view: target, onsave, ontest, onremove: vi.fn() } });
    await fireEvent.click(screen.getByRole('button', { name: 'Test upload' }));
    expect((await screen.findByRole('status')).textContent?.trim()).toBe(
      'Test failed: Bad credentials',
    );
  });

  it('asks before removing the destination', async () => {
    const confirm = vi.spyOn(window, 'confirm').mockReturnValue(true);
    const { onremove } = form(view({ configured: true, status: 'configured' }));
    await fireEvent.click(screen.getByRole('button', { name: 'Remove destination' }));
    expect(confirm).toHaveBeenCalledWith('Remove this destination?');
    await waitFor(() => expect(onremove).toHaveBeenCalledWith('gist'));
  });

  it('does not remove the destination when the confirmation is cancelled', async () => {
    vi.spyOn(window, 'confirm').mockReturnValue(false);
    const { onremove } = form(view({ configured: true, status: 'configured' }));
    await fireEvent.click(screen.getByRole('button', { name: 'Remove destination' }));
    expect(onremove).not.toHaveBeenCalled();
  });
});
