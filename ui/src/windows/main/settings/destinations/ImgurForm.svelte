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

  function initialSaved(source: DestinationView): boolean {
    return source.clientIdSaved;
  }

  let clientId = $state('');
  let clientSaved = $state(untrack(() => initialSaved(view)));
  let error = $state('');
  let saving = $state(false);
  let removing = $state(false);
  let testState = $state<'idle' | 'testing' | 'success' | 'failed'>('idle');
  let testMessage = $state<string | null>(null);

  async function save(): Promise<void> {
    if (clientId.trim() === '' && !clientSaved) {
      error = 'Enter the Imgur Client ID.';
      return;
    }
    error = '';
    saving = true;
    try {
      const updated = await onsave({
        id: view.id,
        kind: 'imgur',
        clientId: clientId.trim() === '' ? null : clientId.trim(),
        headers: [],
      });
      clientSaved = updated.clientIdSaved;
      clientId = '';
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

<div class="destination-form" aria-label="Imgur">
  <div class="fields">
    <SecretInput
      label="Client ID"
      bind:value={clientId}
      saved={clientSaved}
      oninput={() => (error = '')}
    />
  </div>
  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}
  <div class="footer">
    <Button variant="accent" height={34} icon="upload" onclick={test} disabled={testState === 'testing'}>
      Test upload
    </Button>
    <Button variant="secondary" height={34} onclick={save} disabled={saving}>Save</Button>
    <Button variant="danger-text" height={34} onclick={remove} disabled={removing}>
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
