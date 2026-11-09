<script lang="ts">
  import { untrack } from 'svelte';
  import Button from '../../../../components/Button.svelte';
  import Segmented from '../../../../components/Segmented.svelte';
  import type {
    DestinationInput,
    DestinationTestOutcome,
    DestinationView,
  } from '../../../../lib/ipc/upload';
  import SecretInput from './SecretInput.svelte';
  import TestStatus from './TestStatus.svelte';
  import { errorText } from './model';

  type SftpView = DestinationView;

  interface SftpInput extends DestinationInput {
    host?: string | null;
    port?: number | null;
    username?: string | null;
    authMethod?: string | null;
    privateKeyPath?: string | null;
    remoteFolder?: string | null;
    password?: string | null;
    passphrase?: string | null;
  }

  interface Props {
    view: SftpView;
    onsave: (input: DestinationInput) => Promise<DestinationView>;
    ontest: (id: string) => Promise<DestinationTestOutcome>;
    onremove: (id: string) => Promise<void>;
  }

  const PASSWORD = 'Password';
  const PRIVATE_KEY = 'Private key file';
  const AUTH_OPTIONS = [PASSWORD, PRIVATE_KEY];

  let { view, onsave, ontest, onremove }: Props = $props();

  const start = untrack(() => ({
    host: view.host ?? '',
    port: String(view.port ?? 22),
    username: view.username ?? '',
    auth: view.authMethod === 'private_key' ? PRIVATE_KEY : PASSWORD,
    privateKeyPath: view.privateKeyPath ?? '',
    remoteFolder: view.remoteFolder ?? '',
    publicUrlPattern: view.publicUrlPattern ?? '',
    passwordSaved: view.passwordSaved ?? false,
    passphraseSaved: view.passphraseSaved ?? false,
  }));

  let host = $state(start.host);
  let port = $state(start.port);
  let username = $state(start.username);
  let auth = $state<string>(start.auth);
  let privateKeyPath = $state(start.privateKeyPath);
  let remoteFolder = $state(start.remoteFolder);
  let publicUrlPattern = $state(start.publicUrlPattern);
  let password = $state('');
  let passphrase = $state('');
  let passwordSaved = $state(start.passwordSaved);
  let passphraseSaved = $state(start.passphraseSaved);
  let error = $state('');
  let saving = $state(false);
  let removing = $state(false);
  let testState = $state<'idle' | 'testing' | 'success' | 'failed'>('idle');
  let testMessage = $state<string | null>(null);

  async function save(): Promise<void> {
    const trimmedHost = host.trim();
    const trimmedUsername = username.trim();
    const portNumber = Number(port.trim());
    if (trimmedHost === '') {
      error = 'Enter the SFTP host.';
      return;
    }
    if (!Number.isInteger(portNumber) || portNumber < 1 || portNumber > 65535) {
      error = 'The SFTP port must be between 1 and 65535.';
      return;
    }
    if (trimmedUsername === '') {
      error = 'Enter the SFTP username.';
      return;
    }
    if (auth === PRIVATE_KEY && privateKeyPath.trim() === '') {
      error = 'Choose the SFTP private key file.';
      return;
    }
    if (!publicUrlPattern.includes('<key>')) {
      error = 'The public URL pattern must contain <key>.';
      return;
    }
    if (auth === PASSWORD && password === '' && !passwordSaved) {
      error = 'Enter the SFTP password.';
      return;
    }
    error = '';
    saving = true;
    try {
      const input: SftpInput = {
        id: view.id,
        kind: 'sftp',
        host: trimmedHost,
        port: portNumber,
        username: trimmedUsername,
        authMethod: auth === PRIVATE_KEY ? 'private_key' : 'password',
        privateKeyPath: auth === PRIVATE_KEY ? privateKeyPath.trim() : null,
        password: auth === PASSWORD && password !== '' ? password : null,
        passphrase: auth === PRIVATE_KEY && passphrase !== '' ? passphrase : null,
        remoteFolder: remoteFolder.trim(),
        publicUrlPattern: publicUrlPattern.trim(),
        headers: [],
      };
      const updated = (await onsave(input)) as SftpView;
      passwordSaved = updated.passwordSaved ?? false;
      passphraseSaved = updated.passphraseSaved ?? false;
      password = '';
      passphrase = '';
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

<div class="destination-form" role="group" aria-label="SFTP">
  <div class="fields">
    <label class="field">
      <span class="label">Host</span>
      <input
        type="text"
        spellcheck="false"
        autocomplete="off"
        bind:value={host}
        oninput={() => (error = '')}
      />
    </label>
    <label class="field">
      <span class="label">Port (22)</span>
      <input
        type="text"
        inputmode="numeric"
        spellcheck="false"
        autocomplete="off"
        bind:value={port}
        oninput={() => (error = '')}
      />
    </label>
    <label class="field">
      <span class="label">Username</span>
      <input
        type="text"
        spellcheck="false"
        autocomplete="off"
        bind:value={username}
        oninput={() => (error = '')}
      />
    </label>
  </div>
  <div class="choice">
    <span class="label">Password or Private key file</span>
    <Segmented
      options={AUTH_OPTIONS}
      value={auth}
      label="Password or Private key file"
      onchange={(value) => {
        auth = value;
        error = '';
      }}
    />
  </div>
  {#if auth === PASSWORD}
    <div class="fields">
      <SecretInput
        label="Password"
        bind:value={password}
        saved={passwordSaved}
        oninput={() => (error = '')}
      />
    </div>
  {:else}
    <div class="fields">
      <label class="field">
        <span class="label">Private key file</span>
        <input
          type="text"
          class="mono"
          spellcheck="false"
          autocomplete="off"
          bind:value={privateKeyPath}
          oninput={() => (error = '')}
        />
      </label>
      <SecretInput
        label="Passphrase"
        bind:value={passphrase}
        saved={passphraseSaved}
        oninput={() => (error = '')}
      />
    </div>
  {/if}
  <div class="fields">
    <label class="field">
      <span class="label">Remote folder</span>
      <input
        type="text"
        class="mono"
        spellcheck="false"
        autocomplete="off"
        bind:value={remoteFolder}
        oninput={() => (error = '')}
      />
    </label>
    <label class="field wide">
      <span class="label">Public URL pattern</span>
      <input
        type="text"
        class="mono"
        spellcheck="false"
        autocomplete="off"
        placeholder="https://example.com/<key>"
        bind:value={publicUrlPattern}
        oninput={() => (error = '')}
      />
    </label>
  </div>
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

  .choice {
    display: flex;
    flex-direction: column;
    gap: 6px;
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
