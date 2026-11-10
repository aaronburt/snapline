import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import type { DestinationView } from '../../../../lib/ipc/upload';
import { expectAccessible } from '../../../../test/axe';
import CustomForm from './CustomForm.svelte';
import ImgurForm from './ImgurForm.svelte';
import S3Form from './S3Form.svelte';

function view(overrides: Partial<DestinationView> = {}): DestinationView {
  return {
    id: 'destination',
    kind: 'custom',
    name: 'Custom uploader',
    detail: '',
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
    method: 'POST',
    requestUrl: '',
    body: 'MultipartFormData',
    fileFormField: 'file',
    link: '',
    thumbnail: null,
    deletion: null,
    errorMessage: null,
    headers: [],
    importedFromSxcu: false,
    host: null,
    port: null,
    username: null,
    authMethod: null,
    privateKeyPath: null,
    remoteFolder: null,
    passwordSaved: false,
    passphraseSaved: false,
    warnings: [],
    ...overrides,
  };
}

function props(target: DestinationView, saved: DestinationView = target) {
  return {
    view: target,
    onsave: vi.fn().mockResolvedValue(saved),
    ontest: vi
      .fn()
      .mockResolvedValue({ status: 'succeeded', link: 'https://x.test/a', message: null }),
    onremove: vi.fn().mockResolvedValue(undefined),
  };
}

function customProps(target: DestinationView, saved: DestinationView = target) {
  return {
    ...props(target, saved),
    onexport: vi.fn().mockResolvedValue('C:\\uploader.sxcu'),
  };
}

async function type(label: string, value: string): Promise<HTMLInputElement> {
  const input = screen.getByLabelText(label) as HTMLInputElement;
  await fireEvent.input(input, { target: { value } });
  return input;
}

async function click(name: string): Promise<void> {
  await fireEvent.click(screen.getByRole('button', { name }));
}

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe('destination form accessibility', () => {
  it('Custom uploader', async () => {
    const { container } = render(CustomForm, { props: customProps(view()) });
    await expectAccessible(container);
  });

  it('Imgur', async () => {
    const { container } = render(ImgurForm, {
      props: props(view({ id: 'imgur', kind: 'imgur', name: 'Imgur' })),
    });
    await expectAccessible(container);
  });

  it('Amazon S3', async () => {
    const { container } = render(S3Form, {
      props: props(view({ id: 's3', kind: 's3', name: 'Amazon S3' })),
    });
    await expectAccessible(container);
  });
});

describe('ImgurForm', () => {
  const imgur = view({ id: 'imgur', kind: 'imgur', name: 'Imgur' });
  const savedImgur = view({ id: 'imgur', kind: 'imgur', name: 'Imgur', clientIdSaved: true });

  it('requires a Client ID when none is saved', async () => {
    const handlers = props(imgur);
    render(ImgurForm, { props: handlers });
    await click('Save');
    expect(handlers.onsave).not.toHaveBeenCalled();
    expect(screen.getByRole('alert').textContent).toBe('Enter the Imgur Client ID.');
    await type('Client ID', 'abc');
    expect(screen.queryByRole('alert')).toBeNull();
  });

  it('saves the trimmed Client ID, then clears the field', async () => {
    const handlers = props(imgur, savedImgur);
    render(ImgurForm, { props: handlers });
    const input = await type('Client ID', '  abc123  ');
    await click('Save');
    await waitFor(() =>
      expect(handlers.onsave).toHaveBeenCalledWith({
        id: 'imgur',
        kind: 'imgur',
        clientId: 'abc123',
        headers: [],
      }),
    );
    await waitFor(() => expect(input.value).toBe(''));
    expect(input.placeholder).toBe('Saved');
  });

  it('keeps a saved Client ID by sending null', async () => {
    const handlers = props(savedImgur);
    render(ImgurForm, { props: handlers });
    await click('Save');
    await waitFor(() =>
      expect(handlers.onsave).toHaveBeenCalledWith({
        id: 'imgur',
        kind: 'imgur',
        clientId: null,
        headers: [],
      }),
    );
  });

  it('shows a save failure', async () => {
    const handlers = props(savedImgur);
    handlers.onsave.mockRejectedValue({ message: 'The settings file is read-only.' });
    render(ImgurForm, { props: handlers });
    await click('Save');
    expect((await screen.findByRole('alert')).textContent).toBe('The settings file is read-only.');
  });

  it('reports test upload results and failures', async () => {
    const handlers = props(savedImgur);
    render(ImgurForm, { props: handlers });
    await click('Test upload');
    await waitFor(() => expect(handlers.ontest).toHaveBeenCalledWith('imgur'));
    expect((await screen.findByRole('status')).textContent?.trim()).toBe(
      'Test upload succeeded · link copied',
    );
    handlers.ontest.mockRejectedValue({ message: 'No network.' });
    await click('Test upload');
    await waitFor(() =>
      expect(screen.getByRole('status').textContent?.trim()).toBe('Test failed: No network.'),
    );
  });

  it('removes only after confirmation', async () => {
    const confirm = vi.spyOn(window, 'confirm').mockReturnValue(false);
    const handlers = props(savedImgur);
    render(ImgurForm, { props: handlers });
    await click('Remove destination');
    expect(handlers.onremove).not.toHaveBeenCalled();
    confirm.mockReturnValue(true);
    await click('Remove destination');
    await waitFor(() => expect(handlers.onremove).toHaveBeenCalledWith('imgur'));
  });
});

describe('S3Form', () => {
  const empty = view({ id: 's3', kind: 's3', name: 'Amazon S3' });
  const configured = view({
    id: 's3',
    kind: 's3',
    name: 'Amazon S3',
    endpoint: 'https://s3.example.test',
    region: 'eu-west-2',
    bucket: 'shots',
    pathPrefix: 'captures/',
    publicUrlPattern: 'https://cdn.example.test/<key>',
    aclPublicRead: true,
    accessKeyIdSaved: true,
    secretAccessKeySaved: true,
  });

  it('asks for each missing field in turn', async () => {
    const handlers = props(empty);
    render(S3Form, { props: handlers });
    const error = (): string | null => screen.getByRole('alert').textContent;
    await click('Save');
    expect(error()).toBe('Enter the S3 region.');
    await type('Region', 'eu-west-2');
    await click('Save');
    expect(error()).toBe('Enter the S3 bucket.');
    await type('Bucket', 'shots');
    await click('Save');
    expect(error()).toBe('Enter the S3 access key ID.');
    await type('Access key ID', 'AKIAEXAMPLE');
    await click('Save');
    expect(error()).toBe('Enter the S3 secret access key.');
    await type('Secret access key', 'secret');
    await type('Public URL pattern', 'https://cdn.example.test/');
    await click('Save');
    expect(error()).toBe('The public URL pattern must contain <key>.');
    expect(handlers.onsave).not.toHaveBeenCalled();
  });

  it('saves new credentials with the default AWS pattern, then clears them', async () => {
    const handlers = props(empty, configured);
    render(S3Form, { props: handlers });
    await type('Region', ' eu-west-2 ');
    await type('Bucket', ' shots ');
    await type('Path prefix', 'captures/');
    const access = await type('Access key ID', ' AKIAEXAMPLE ');
    const secret = await type('Secret access key', ' secret ');
    await click('Save');
    await waitFor(() =>
      expect(handlers.onsave).toHaveBeenCalledWith({
        id: 's3',
        kind: 's3',
        endpoint: null,
        region: 'eu-west-2',
        bucket: 'shots',
        accessKeyId: 'AKIAEXAMPLE',
        secretAccessKey: 'secret',
        pathPrefix: 'captures/',
        publicUrlPattern: 'https://<bucket>.s3.<region>.amazonaws.com/<key>',
        aclPublicRead: false,
        headers: [],
      }),
    );
    await waitFor(() => expect(secret.value).toBe(''));
    expect(access.value).toBe('');
    expect(secret.placeholder).toBe('Saved');
    expect(access.placeholder).toBe('Saved');
  });

  it('keeps saved credentials by sending null and never echoes them', async () => {
    const handlers = props(configured);
    render(S3Form, { props: handlers });
    expect((screen.getByLabelText('Secret access key') as HTMLInputElement).value).toBe('');
    expect((screen.getByLabelText('Access key ID') as HTMLInputElement).value).toBe('');
    await fireEvent.click(screen.getByRole('checkbox'));
    await click('Save');
    await waitFor(() =>
      expect(handlers.onsave).toHaveBeenCalledWith({
        id: 's3',
        kind: 's3',
        endpoint: 'https://s3.example.test',
        region: 'eu-west-2',
        bucket: 'shots',
        accessKeyId: null,
        secretAccessKey: null,
        pathPrefix: 'captures/',
        publicUrlPattern: 'https://cdn.example.test/<key>',
        aclPublicRead: false,
        headers: [],
      }),
    );
  });

  it('runs a test upload and removes after confirmation', async () => {
    vi.spyOn(window, 'confirm').mockReturnValue(true);
    const handlers = props(configured);
    handlers.ontest.mockResolvedValue({ status: 'failed', link: null, message: 'Access denied' });
    render(S3Form, { props: handlers });
    await click('Test upload');
    await waitFor(() => expect(handlers.ontest).toHaveBeenCalledWith('s3'));
    expect((await screen.findByRole('status')).textContent?.trim()).toBe(
      'Test failed: Access denied',
    );
    await click('Remove destination');
    await waitFor(() => expect(handlers.onremove).toHaveBeenCalledWith('s3'));
  });
});

describe('CustomForm', () => {
  const configured = view({
    id: 'custom-1',
    name: 'My host',
    method: 'PUT',
    requestUrl: 'https://up.example.test/files',
    body: 'Binary',
    fileFormField: '',
    link: '{json:$.data.url}',
    thumbnail: '{json:$.data.thumb}',
    deletion: null,
    errorMessage: null,
    headers: [{ name: 'Authorization', saved: true }],
    importedFromSxcu: true,
  });

  it('asks for the request URL, form field and link', async () => {
    const handlers = customProps(view());
    render(CustomForm, { props: handlers });
    const error = (): string | null => screen.getByRole('alert').textContent;
    await click('Save');
    expect(error()).toBe('Enter the request URL.');
    await type('Request URL', 'https://up.example.test/files');
    await type('File form field', '  ');
    await click('Save');
    expect(error()).toBe('Enter the file form field.');
    await type('File form field', 'file');
    await click('Save');
    expect(error()).toBe('Enter the link from the response.');
    expect(handlers.onsave).not.toHaveBeenCalled();
  });

  it('saves a new uploader, wrapping JSON paths and sending new header values', async () => {
    const handlers = customProps(
      view(),
      view({ name: 'Renamed', headers: [{ name: 'X-Token', saved: true }] }),
    );
    render(CustomForm, { props: handlers });
    const name = await type('Name', ' Renamed ');
    await type('Request URL', ' https://up.example.test/files ');
    await type('Link from response', '$.data.url');
    await type('Deletion link from response', '{response}');
    await click('Add header');
    await type('Header name', 'X-Token');
    const value = await type('Header value', 's3cret');
    await click('Save');
    await waitFor(() =>
      expect(handlers.onsave).toHaveBeenCalledWith({
        id: 'destination',
        kind: 'custom',
        name: 'Renamed',
        method: 'POST',
        requestUrl: 'https://up.example.test/files',
        body: 'MultipartFormData',
        fileFormField: 'file',
        link: '{json:$.data.url}',
        thumbnail: null,
        deletion: '{response}',
        errorMessage: null,
        headers: [{ name: 'X-Token', value: 's3cret' }],
        importedFromSxcu: false,
      }),
    );
    await waitFor(() => expect(name.value).toBe('Renamed'));
    const saved = screen.getByLabelText('Header value') as HTMLInputElement;
    expect(saved.value).toBe('');
    expect(saved.placeholder).toBe('Saved');
    expect(value.isConnected ? value.value : '').toBe('');
  });

  it('keeps saved header values by sending null and shows stored paths without the wrapper', async () => {
    const handlers = customProps(configured);
    render(CustomForm, { props: handlers });
    expect((screen.getByLabelText('Link from response') as HTMLInputElement).value).toBe(
      '$.data.url',
    );
    expect((screen.getByLabelText('Header value') as HTMLInputElement).value).toBe('');
    await click('Save');
    await waitFor(() =>
      expect(handlers.onsave).toHaveBeenCalledWith({
        id: 'custom-1',
        kind: 'custom',
        name: 'My host',
        method: 'PUT',
        requestUrl: 'https://up.example.test/files',
        body: 'Binary',
        fileFormField: '',
        link: '{json:$.data.url}',
        thumbnail: '{json:$.data.thumb}',
        deletion: null,
        errorMessage: null,
        headers: [{ name: 'Authorization', value: null }],
        importedFromSxcu: true,
      }),
    );
  });

  it('exports without header values unless they are asked for', async () => {
    const handlers = customProps(configured);
    render(CustomForm, { props: handlers });
    await click('Export .sxcu');
    await waitFor(() => expect(handlers.onexport).toHaveBeenLastCalledWith('custom-1', false));
    await fireEvent.click(screen.getByRole('checkbox', { name: 'Include header values' }));
    await click('Export .sxcu');
    await waitFor(() => expect(handlers.onexport).toHaveBeenLastCalledWith('custom-1', true));
  });

  it('shows an export failure, runs a test upload and removes after confirmation', async () => {
    vi.spyOn(window, 'confirm').mockReturnValue(true);
    const handlers = customProps(configured);
    handlers.onexport.mockRejectedValue({ message: 'The file could not be written.' });
    render(CustomForm, { props: handlers });
    await click('Export .sxcu');
    expect((await screen.findByRole('alert')).textContent).toBe('The file could not be written.');
    await click('Test upload');
    await waitFor(() => expect(handlers.ontest).toHaveBeenCalledWith('custom-1'));
    await click('Remove destination');
    await waitFor(() => expect(handlers.onremove).toHaveBeenCalledWith('custom-1'));
  });
});
