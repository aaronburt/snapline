import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render } from '@testing-library/svelte';
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

function props(target: DestinationView) {
  return {
    view: target,
    onsave: vi.fn(),
    ontest: vi.fn(),
    onremove: vi.fn(),
  };
}

function customProps(target: DestinationView) {
  return {
    ...props(target),
    onexport: vi.fn(),
  };
}

afterEach(cleanup);

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
