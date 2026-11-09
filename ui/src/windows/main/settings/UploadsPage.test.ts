import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import UploadsPage from './UploadsPage.svelte';
import { resetSettings } from './settings';
import { settingsDto } from './settingsDto';
import type { DestinationView } from '../../../lib/ipc/upload';

const settingsIpc = vi.hoisted(() => ({
  ping: vi.fn(),
  get: vi.fn(),
  update: vi.fn(),
  previewName: vi.fn(),
  browseFolder: vi.fn(),
}));

const uploadIpc = vi.hoisted(() => ({
  ping: vi.fn(),
  listDestinations: vi.fn(),
  addDestination: vi.fn(),
  saveDestination: vi.fn(),
  removeDestination: vi.fn(),
  testDestination: vi.fn(),
  importSxcu: vi.fn(),
  exportSxcu: vi.fn(),
}));

vi.mock('../../../lib/ipc/settings', () => settingsIpc);
vi.mock('../../../lib/ipc/upload', () => uploadIpc);

function destination(overrides: Partial<DestinationView> = {}): DestinationView {
  return {
    id: 's3',
    kind: 's3',
    name: 'Amazon S3',
    detail: 'Bucket my-bucket · eu-west-2',
    status: 'not_set_up',
    action: 'set_up',
    configured: false,
    clientIdSaved: false,
    endpoint: null,
    region: '',
    bucket: '',
    pathPrefix: '',
    publicUrlPattern: 'https://<bucket>.s3.<region>.amazonaws.com/<key>',
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

beforeEach(() => {
  resetSettings();
  settingsIpc.get.mockReset();
  settingsIpc.update.mockReset();
  settingsIpc.get.mockResolvedValue(settingsDto());
  settingsIpc.update.mockResolvedValue(settingsDto());
  uploadIpc.listDestinations.mockReset();
  uploadIpc.addDestination.mockReset();
  uploadIpc.saveDestination.mockReset();
  uploadIpc.removeDestination.mockReset();
  uploadIpc.testDestination.mockReset();
  uploadIpc.importSxcu.mockReset();
  uploadIpc.exportSxcu.mockReset();
  uploadIpc.listDestinations.mockResolvedValue([]);
  uploadIpc.importSxcu.mockResolvedValue({ destination: null, warnings: [] });
});

afterEach(cleanup);

describe('UploadsPage', () => {
  it('gets the settings on mount and defaults every destination', async () => {
    render(UploadsPage);
    for (const title of ['Images', 'Videos and GIFs', 'Text and code', 'Other files']) {
      expect((screen.getByRole('combobox', { name: title }) as HTMLSelectElement).value).toBe(
        'save_locally_only',
      );
    }
    expect(
      (screen.getByRole('combobox', { name: 'Link shortener' }) as HTMLSelectElement).value,
    ).toBe('none');
    await waitFor(() => expect(settingsIpc.get).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(uploadIpc.listDestinations).toHaveBeenCalledTimes(1));
  });

  it('shows the account buttons and the empty destination list', () => {
    render(UploadsPage);
    expect(screen.getByRole('button', { name: /Import ShareX uploader \(\.sxcu\)/ })).not.toBeNull();
    expect(screen.getByRole('button', { name: /Add destination/ })).not.toBeNull();
    expect(screen.getByText('No destinations yet')).not.toBeNull();
  });

  it('shows the upload behaviour defaults', () => {
    render(UploadsPage);
    expect(
      (screen.getByRole('combobox', { name: 'Retry failed uploads' }) as HTMLSelectElement).value,
    ).toBe('3 times');
    expect(screen.getByRole('radio', { name: '60 s' }).getAttribute('aria-checked')).toBe('true');
    expect(
      (screen.getByRole('combobox', { name: 'Keep upload history' }) as HTMLSelectElement).value,
    ).toBe('Forever');
    expect((screen.getByRole('combobox', { name: 'Proxy' }) as HTMLSelectElement).value).toBe(
      'Use system proxy',
    );
  });

  it('sends a select change to the service', async () => {
    render(UploadsPage);
    await fireEvent.change(screen.getByRole('combobox', { name: 'Retry failed uploads' }), {
      target: { value: 'Once' },
    });
    await waitFor(() =>
      expect(settingsIpc.update).toHaveBeenCalledWith({ uploads: { retry: 'once' } }),
    );
  });

  it('lists configured destinations with a status and opens the matching form', async () => {
    uploadIpc.listDestinations.mockResolvedValue([
      destination({
        id: 'imgur',
        kind: 'imgur',
        name: 'Imgur',
        detail: 'Image host · anonymous uploads',
        status: 'configured',
        action: 'configure',
        configured: true,
        clientIdSaved: true,
      }),
    ]);
    render(UploadsPage);
    expect(await screen.findByText('Imgur')).not.toBeNull();
    expect(screen.getByText('Image host · anonymous uploads')).not.toBeNull();
    expect(screen.getByText('Configured')).not.toBeNull();
    await fireEvent.click(screen.getByRole('button', { name: 'Configure' }));
    expect(screen.getByRole('heading', { name: 'Imgur' })).not.toBeNull();
    const clientId = screen.getByLabelText('Client ID') as HTMLInputElement;
    expect(clientId.value).toBe('');
    expect(clientId.placeholder).toBe('Saved');
  });

  it('adds an Amazon S3 destination from the native menu and saves its form', async () => {
    const blank = destination();
    const configured = destination({
      status: 'configured',
      action: 'configure',
      configured: true,
      region: 'eu-west-2',
      bucket: 'my-bucket',
      accessKeyIdSaved: true,
      secretAccessKeySaved: true,
      detail: 'Bucket my-bucket · eu-west-2',
    });
    uploadIpc.addDestination.mockResolvedValue(blank);
    uploadIpc.saveDestination.mockResolvedValue(configured);
    render(UploadsPage);
    await fireEvent.click(screen.getByRole('button', { name: /Add destination/ }));
    await fireEvent.click(screen.getByRole('button', { name: 'Amazon S3' }));
    await waitFor(() => expect(uploadIpc.addDestination).toHaveBeenCalledWith('s3'));
    expect(await screen.findByRole('heading', { name: 'Amazon S3' })).not.toBeNull();

    await fireEvent.input(screen.getByLabelText('Region'), { target: { value: 'eu-west-2' } });
    await fireEvent.input(screen.getByLabelText('Bucket'), { target: { value: 'my-bucket' } });
    await fireEvent.input(screen.getByLabelText('Access key ID'), {
      target: { value: 'AKIDEXAMPLE' },
    });
    await fireEvent.input(screen.getByLabelText('Secret access key'), {
      target: { value: 'top-secret-key' },
    });
    await fireEvent.click(screen.getByRole('button', { name: 'Save' }));

    await waitFor(() =>
      expect(uploadIpc.saveDestination).toHaveBeenCalledWith(
        expect.objectContaining({
          id: 's3',
          kind: 's3',
          region: 'eu-west-2',
          bucket: 'my-bucket',
          accessKeyId: 'AKIDEXAMPLE',
          secretAccessKey: 'top-secret-key',
        }),
      ),
    );
    expect(await screen.findByText('Configured')).not.toBeNull();
  });

  it('shows the exact test upload pill texts', async () => {
    const blank = destination();
    uploadIpc.addDestination.mockResolvedValue(blank);
    uploadIpc.testDestination.mockResolvedValue({
      status: 'succeeded',
      link: 'https://example.test/snapline-test.png',
      message: null,
    });
    render(UploadsPage);
    await fireEvent.click(screen.getByRole('button', { name: /Add destination/ }));
    await fireEvent.click(screen.getByRole('button', { name: 'Amazon S3' }));
    expect(await screen.findByText('Not tested yet')).not.toBeNull();
    await fireEvent.click(screen.getByRole('button', { name: 'Test upload' }));
    expect(await screen.findByText('Test upload succeeded · link copied')).not.toBeNull();
  });

  it('shows a failed test upload with the message', async () => {
    uploadIpc.addDestination.mockResolvedValue(destination());
    uploadIpc.testDestination.mockResolvedValue({
      status: 'failed',
      link: null,
      message: 'The server returned HTTP 401.',
    });
    render(UploadsPage);
    await fireEvent.click(screen.getByRole('button', { name: /Add destination/ }));
    await fireEvent.click(screen.getByRole('button', { name: 'Amazon S3' }));
    await fireEvent.click(await screen.findByRole('button', { name: 'Test upload' }));
    expect(
      await screen.findByText('Test failed: The server returned HTTP 401.'),
    ).not.toBeNull();
  });

  it('imports a ShareX uploader into the card list', async () => {
    uploadIpc.importSxcu.mockResolvedValue({
      destination: destination({
        id: 'custom',
        kind: 'custom',
        name: 'My server',
        detail: 'Custom uploader · imported from .sxcu',
        status: 'configured',
        action: 'edit',
        configured: true,
        method: 'POST',
        requestUrl: 'https://example.test/upload',
        body: 'MultipartFormData',
        fileFormField: 'file',
        link: '{json:$.data.url}',
        importedFromSxcu: true,
      }),
      warnings: [],
    });
    render(UploadsPage);
    await fireEvent.click(screen.getByRole('button', { name: /Import ShareX uploader/ }));
    await waitFor(() => expect(uploadIpc.importSxcu).toHaveBeenCalledTimes(1));
    expect(await screen.findByText('My server')).not.toBeNull();
    expect(screen.getByText('Custom uploader · imported from .sxcu')).not.toBeNull();
    expect(screen.getByRole('button', { name: 'Edit' })).not.toBeNull();
  });

  it('removes a destination after confirmation', async () => {
    uploadIpc.listDestinations.mockResolvedValue([
      destination({
        status: 'configured',
        action: 'configure',
        configured: true,
      }),
    ]);
    uploadIpc.removeDestination.mockResolvedValue(undefined);
    const confirm = vi.spyOn(window, 'confirm').mockReturnValue(true);
    render(UploadsPage);
    await fireEvent.click(await screen.findByRole('button', { name: 'Configure' }));
    await fireEvent.click(screen.getByRole('button', { name: 'Remove destination' }));
    await waitFor(() => expect(uploadIpc.removeDestination).toHaveBeenCalledWith('s3'));
    expect(screen.queryByText('Bucket my-bucket · eu-west-2')).toBeNull();
    confirm.mockRestore();
  });
});
