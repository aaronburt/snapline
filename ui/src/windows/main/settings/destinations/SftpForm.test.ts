import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { expectAccessible } from '../../../../test/axe';
import type { DestinationView } from '../../../../lib/ipc/upload';
import SftpForm from './SftpForm.svelte';

afterEach(cleanup);

type SftpView = DestinationView & {
  host?: string | null;
  port?: number | null;
  username?: string | null;
  authMethod?: string | null;
  privateKeyPath?: string | null;
  remoteFolder?: string | null;
  passwordSaved?: boolean;
  passphraseSaved?: boolean;
};

function view(overrides: Partial<SftpView> = {}): SftpView {
  return {
    id: 'sftp',
    kind: 'sftp',
    name: 'SFTP',
    detail: 'Your own server over SSH',
    status: 'not_set_up' as const,
    action: 'set_up' as const,
    configured: false,
    clientIdSaved: false,
    endpoint: null,
    region: null,
    bucket: null,
    pathPrefix: null,
    publicUrlPattern: 'https://example.com/<key>',
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
    host: 'example.com',
    port: 22,
    username: 'aaron',
    authMethod: 'password',
    privateKeyPath: null,
    remoteFolder: '/srv/shots',
    passwordSaved: false,
    passphraseSaved: false,
    ...overrides,
  };
}

describe('SftpForm', () => {
  it('shows the seven settings fields and the default port', async () => {
    const { container } = render(SftpForm, {
      props: {
        view: view(),
        onsave: vi.fn(),
        ontest: vi.fn(),
        onremove: vi.fn(),
      },
    });
    await expectAccessible(container);
    expect((screen.getByLabelText('Host') as HTMLInputElement).value).toBe('example.com');
    expect((screen.getByLabelText('Port (22)') as HTMLInputElement).value).toBe('22');
    expect(screen.getByLabelText('Username')).not.toBeNull();
    expect(screen.getByRole('radiogroup', { name: 'Password or Private key file' })).not.toBeNull();
    expect(screen.getByLabelText('Password')).not.toBeNull();
    expect((screen.getByLabelText('Remote folder') as HTMLInputElement).value).toBe('/srv/shots');
    expect(screen.getByLabelText('Public URL pattern')).not.toBeNull();
    expect(screen.getByRole('button', { name: 'Test upload' })).not.toBeNull();
    expect(screen.getByRole('button', { name: 'Save' })).not.toBeNull();
    expect(screen.getByRole('button', { name: 'Remove destination' })).not.toBeNull();
  });

  it('maps password settings to the destination input', async () => {
    const onsave = vi.fn().mockResolvedValue(view({ passwordSaved: true }));
    render(SftpForm, {
      props: {
        view: view(),
        onsave,
        ontest: vi.fn(),
        onremove: vi.fn(),
      },
    });
    await fireEvent.input(screen.getByLabelText('Host'), {
      target: { value: 'sftp.example.test' },
    });
    await fireEvent.input(screen.getByLabelText('Port (22)'), { target: { value: '2222' } });
    await fireEvent.input(screen.getByLabelText('Username'), { target: { value: 'snapline' } });
    await fireEvent.input(screen.getByLabelText('Password'), {
      target: { value: 'correct horse' },
    });
    await fireEvent.input(screen.getByLabelText('Remote folder'), {
      target: { value: '/srv/uploads' },
    });
    await fireEvent.input(screen.getByLabelText('Public URL pattern'), {
      target: { value: 'https://cdn.example.test/<key>' },
    });
    await fireEvent.click(screen.getByRole('button', { name: 'Save' }));

    await waitFor(() =>
      expect(onsave).toHaveBeenCalledWith(
        expect.objectContaining({
          id: 'sftp',
          kind: 'sftp',
          host: 'sftp.example.test',
          port: 2222,
          username: 'snapline',
          authMethod: 'password',
          privateKeyPath: null,
          password: 'correct horse',
          passphrase: null,
          remoteFolder: '/srv/uploads',
          publicUrlPattern: 'https://cdn.example.test/<key>',
        }),
      ),
    );
    expect((screen.getByLabelText('Password') as HTMLInputElement).value).toBe('');
    expect((screen.getByLabelText('Password') as HTMLInputElement).placeholder).toBe('Saved');
  });

  it('maps private key settings and never returns a saved secret', async () => {
    const onsave = vi.fn().mockResolvedValue(view({ passphraseSaved: true }));
    render(SftpForm, {
      props: {
        view: view({ authMethod: 'private_key', privateKeyPath: 'C:\\keys\\id_ed25519' }),
        onsave,
        ontest: vi.fn(),
        onremove: vi.fn(),
      },
    });
    expect((screen.getByLabelText('Private key file') as HTMLInputElement).value).toBe(
      'C:\\keys\\id_ed25519',
    );
    expect((screen.getByLabelText('Passphrase') as HTMLInputElement).value).toBe('');
    await fireEvent.input(screen.getByLabelText('Passphrase'), {
      target: { value: 'key secret' },
    });
    await fireEvent.click(screen.getByRole('button', { name: 'Save' }));

    await waitFor(() =>
      expect(onsave).toHaveBeenCalledWith(
        expect.objectContaining({
          authMethod: 'private_key',
          privateKeyPath: 'C:\\keys\\id_ed25519',
          password: null,
          passphrase: 'key secret',
        }),
      ),
    );
    expect((screen.getByLabelText('Passphrase') as HTMLInputElement).value).toBe('');
    expect((screen.getByLabelText('Passphrase') as HTMLInputElement).placeholder).toBe('Saved');
  });

  it('switches between password and private key fields', async () => {
    render(SftpForm, {
      props: {
        view: view(),
        onsave: vi.fn(),
        ontest: vi.fn(),
        onremove: vi.fn(),
      },
    });
    expect(screen.getByLabelText('Password')).not.toBeNull();
    await fireEvent.click(screen.getByRole('radio', { name: 'Private key file' }));
    expect(screen.queryByLabelText('Password')).toBeNull();
    expect(screen.getByLabelText('Private key file')).not.toBeNull();
    expect(screen.getByLabelText('Passphrase')).not.toBeNull();
  });

  it('rejects an invalid port before saving', async () => {
    const onsave = vi.fn();
    render(SftpForm, {
      props: {
        view: view(),
        onsave,
        ontest: vi.fn(),
        onremove: vi.fn(),
      },
    });
    await fireEvent.input(screen.getByLabelText('Port (22)'), { target: { value: '0' } });
    await fireEvent.click(screen.getByRole('button', { name: 'Save' }));
    expect(screen.getByRole('alert').textContent).toContain('between 1 and 65535');
    expect(onsave).not.toHaveBeenCalled();
  });

  it('requires a saved or new password', async () => {
    const onsave = vi.fn().mockResolvedValue(view({ passwordSaved: true }));
    render(SftpForm, {
      props: {
        view: view({ passwordSaved: true }),
        onsave,
        ontest: vi.fn(),
        onremove: vi.fn(),
      },
    });
    expect((screen.getByLabelText('Password') as HTMLInputElement).placeholder).toBe('Saved');
    await fireEvent.click(screen.getByRole('button', { name: 'Save' }));
    await waitFor(() => expect(onsave).toHaveBeenCalled());
    expect(onsave.mock.calls[0][0].password).toBeNull();
  });

  it('requires the <key> token in the public URL pattern', async () => {
    const onsave = vi.fn();
    render(SftpForm, {
      props: {
        view: view(),
        onsave,
        ontest: vi.fn(),
        onremove: vi.fn(),
      },
    });
    await fireEvent.input(screen.getByLabelText('Public URL pattern'), {
      target: { value: 'https://example.test/no-token' },
    });
    await fireEvent.click(screen.getByRole('button', { name: 'Save' }));
    expect(screen.getByRole('alert').textContent).toContain('<key>');
    expect(onsave).not.toHaveBeenCalled();
  });

  it('removes a destination after confirmation', async () => {
    const onremove = vi.fn().mockResolvedValue(undefined);
    const confirm = vi.spyOn(window, 'confirm').mockReturnValue(true);
    render(SftpForm, {
      props: {
        view: view(),
        onsave: vi.fn(),
        ontest: vi.fn(),
        onremove,
      },
    });
    await fireEvent.click(screen.getByRole('button', { name: 'Remove destination' }));
    await waitFor(() => expect(onremove).toHaveBeenCalledWith('sftp'));
    confirm.mockRestore();
  });
});
