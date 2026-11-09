<script lang="ts">
  import { untrack } from 'svelte';
  import Button from '../../../../components/Button.svelte';
  import type {
    DestinationInput,
    DestinationTestOutcome,
    DestinationView,
  } from '../../../../lib/ipc/upload';
  import SecretInput from './SecretInput.svelte';
  import TestStatus from './TestStatus.svelte';
  import { errorText } from './model';

  interface Props {
    view: DestinationView;
    onsave: (input: DestinationInput) => Promise<DestinationView>;
    ontest: (id: string) => Promise<DestinationTestOutcome>;
    onremove: (id: string) => Promise<void>;
  }

  let { view, onsave, ontest, onremove }: Props = $props();

  interface InitialS3 {
    endpoint: string;
    region: string;
    bucket: string;
    pathPrefix: string;
    publicUrlPattern: string;
    aclPublicRead: boolean;
    accessSaved: boolean;
    secretSaved: boolean;
  }

  function initial(source: DestinationView): InitialS3 {
    return {
      endpoint: source.endpoint ?? '',
      region: source.region ?? '',
      bucket: source.bucket ?? '',
      pathPrefix: source.pathPrefix ?? '',
      publicUrlPattern:
        source.publicUrlPattern ?? 'https://<bucket>.s3.<region>.amazonaws.com/<key>',
      aclPublicRead: source.aclPublicRead,
      accessSaved: source.accessKeyIdSaved,
      secretSaved: source.secretAccessKeySaved,
    };
  }

  const start = untrack(() => initial(view));
  let endpoint = $state(start.endpoint);
  let region = $state(start.region);
  let bucket = $state(start.bucket);
  let pathPrefix = $state(start.pathPrefix);
  let publicUrlPattern = $state(start.publicUrlPattern);
  let aclPublicRead = $state(start.aclPublicRead);
  let accessKeyId = $state('');
  let secretAccessKey = $state('');
  let accessSaved = $state(start.accessSaved);
  let secretSaved = $state(start.secretSaved);
  let error = $state('');
  let saving = $state(false);
  let removing = $state(false);
  let testState = $state<'idle' | 'testing' | 'success' | 'failed'>('idle');
  let testMessage = $state<string | null>(null);

  async function save(): Promise<void> {
    if (region.trim() === '') {
      error = 'Enter the S3 region.';
      return;
    }
    if (bucket.trim() === '') {
      error = 'Enter the S3 bucket.';
      return;
    }
    if (accessKeyId.trim() === '' && !accessSaved) {
      error = 'Enter the S3 access key ID.';
      return;
    }
    if (secretAccessKey.trim() === '' && !secretSaved) {
      error = 'Enter the S3 secret access key.';
      return;
    }
    if (!publicUrlPattern.includes('<key>')) {
      error = 'The public URL pattern must contain <key>.';
      return;
    }
    error = '';
    saving = true;
    try {
      const updated = await onsave({
        id: view.id,
        kind: 's3',
        endpoint: endpoint.trim() === '' ? null : endpoint.trim(),
        region: region.trim(),
        bucket: bucket.trim(),
        accessKeyId: accessKeyId.trim() === '' ? null : accessKeyId.trim(),
        secretAccessKey: secretAccessKey.trim() === '' ? null : secretAccessKey.trim(),
        pathPrefix: pathPrefix.trim(),
        publicUrlPattern: publicUrlPattern.trim(),
        aclPublicRead,
        headers: [],
      });
      accessSaved = updated.accessKeyIdSaved;
      secretSaved = updated.secretAccessKeySaved;
      accessKeyId = '';
      secretAccessKey = '';
    } catch (failure) {
      error = errorText(failure);
    } finally {
      saving = false;
    }
  }

  async function test(): Promise<void> {
    testState = 'testing';
    testMessage = null;
    try {
      const outcome = await ontest(view.id);
      testState = outcome.status === 'succeeded' ? 'success' : 'failed';
      testMessage = outcome.message;
    } catch (failure) {
      testState = 'failed';
      testMessage = errorText(failure);
    }
  }

  async function remove(): Promise<void> {
    if (!window.confirm('Remove this destination?')) {
      return;
    }
    error = '';
    removing = true;
    try {
      await onremove(view.id);
    } catch (failure) {
      error = errorText(failure);
    } finally {
      removing = false;
    }
  }
</script>

<div class="destination-form" role="group" aria-label="Amazon S3">
  <div class="fields">
    <label class="field">
      <span class="label">Endpoint (default AWS)</span>
      <input
        type="text"
        placeholder="AWS"
        spellcheck="false"
        bind:value={endpoint}
        oninput={() => (error = '')}
      />
    </label>
    <label class="field">
      <span class="label">Region</span>
      <input type="text" spellcheck="false" bind:value={region} oninput={() => (error = '')} />
    </label>
    <label class="field">
      <span class="label">Bucket</span>
      <input type="text" spellcheck="false" bind:value={bucket} oninput={() => (error = '')} />
    </label>
    <label class="field">
      <span class="label">Path prefix</span>
      <input
        type="text"
        class="mono"
        spellcheck="false"
        bind:value={pathPrefix}
        oninput={() => (error = '')}
      />
    </label>
    <SecretInput
      label="Access key ID"
      bind:value={accessKeyId}
      saved={accessSaved}
      oninput={() => (error = '')}
    />
    <SecretInput
      label="Secret access key"
      bind:value={secretAccessKey}
      saved={secretSaved}
      oninput={() => (error = '')}
    />
    <label class="field wide">
      <span class="label">Public URL pattern</span>
      <input
        type="text"
        class="mono"
        spellcheck="false"
        bind:value={publicUrlPattern}
        oninput={() => (error = '')}
      />
    </label>
  </div>
  <label class="toggle">
    <span class="toggle-text">ACL public-read</span>
    <input type="checkbox" class="tg" bind:checked={aclPublicRead} />
  </label>
  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}
  <div class="footer">
    <Button
      variant="accent"
      height={34}
      icon="upload"
      onclick={test}
      disabled={testState === 'testing'}
      title={testState === 'testing' ? 'A test upload is already running.' : undefined}
    >
      Test upload
    </Button>
    <Button
      variant="secondary"
      height={34}
      onclick={save}
      disabled={saving}
      title={saving ? 'Saving…' : undefined}>Save</Button
    >
    <Button
      variant="danger-text"
      height={34}
      onclick={remove}
      disabled={removing}
      title={removing ? 'Removing…' : undefined}
    >
      Remove destination
    </Button>
    <div class="spacer"></div>
    <TestStatus state={testState} message={testMessage} />
  </div>
</div>

<style>
  .destination-form {
    display: flex;
    flex-direction: column;
    gap: 18px;
  }

  .fields {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(240px, 1fr));
    gap: 14px 16px;
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: 6px;
    min-width: 0;
  }

  .wide {
    grid-column: 1 / -1;
  }

  .label {
    font-size: 12px;
    color: var(--text2);
  }

  input[type='text'] {
    height: 34px;
    padding: 0 10px;
    border: 1px solid var(--stroke);
    border-bottom-color: var(--strokeHi);
    border-radius: 4px;
    background: var(--layer);
    color: var(--text);
    font-size: 12px;
  }

  .mono {
    font-family: var(--type-mono);
  }

  .toggle {
    display: flex;
    align-items: center;
    gap: 16px;
    font-size: 13px;
  }

  .toggle-text {
    color: var(--text2);
  }

  .error {
    margin: 0;
    color: var(--err);
    font-size: 12px;
  }

  .footer {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
    padding-top: 14px;
    border-top: 1px solid var(--stroke);
  }

  .spacer {
    flex: 1;
  }
</style>
