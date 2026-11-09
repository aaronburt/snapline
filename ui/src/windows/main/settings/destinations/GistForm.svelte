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

  const start = untrack(() => ({
    tokenSaved: view.configured,
    publicGist: view.aclPublicRead,
  }));

  let token = $state('');
  let tokenSaved = $state(start.tokenSaved);
  let publicGist = $state(start.publicGist);
  let error = $state('');
  let saving = $state(false);
  let removing = $state(false);
  let testState = $state<'idle' | 'testing' | 'success' | 'failed'>('idle');
  let testMessage = $state<string | null>(null);

  async function save(): Promise<void> {
    if (token.trim() === '' && !tokenSaved) {
      error = 'Enter the GitHub token.';
      return;
    }
    error = '';
    saving = true;
    try {
      const updated = await onsave({
        id: view.id,
        kind: 'gist',
        clientId: token.trim() === '' ? null : token.trim(),
        aclPublicRead: publicGist,
        headers: [],
      });
      tokenSaved = updated.configured;
      publicGist = updated.aclPublicRead;
      token = '';
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

<div class="destination-form" role="group" aria-label="GitHub Gist">
  <div class="fields">
    <SecretInput
      label="Token"
      bind:value={token}
      saved={tokenSaved}
      oninput={(value) => {
        token = value;
        error = '';
      }}
    />
  </div>
  <label class="toggle">
    <span class="toggle-text">Public</span>
    <input type="checkbox" class="tg" bind:checked={publicGist} />
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
