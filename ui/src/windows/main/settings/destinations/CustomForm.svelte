<script lang="ts">
  import { untrack } from 'svelte';
  import Button from '../../../../components/Button.svelte';
  import Icon from '../../../../components/Icon.svelte';
  import type {
    DestinationInput,
    DestinationTestOutcome,
    DestinationView,
  } from '../../../../lib/ipc/upload';
  import SecretInput from './SecretInput.svelte';
  import TestStatus from './TestStatus.svelte';
  import {
    BODY_OPTIONS,
    METHOD_OPTIONS,
    bodyLabel,
    bodyValue,
    errorText,
    pathDisplay,
    pathValue,
  } from './model';

  interface HeaderRow {
    name: string;
    value: string;
    saved: boolean;
  }

  interface Props {
    view: DestinationView;
    onsave: (input: DestinationInput) => Promise<DestinationView>;
    ontest: (id: string) => Promise<DestinationTestOutcome>;
    onremove: (id: string) => Promise<void>;
    onexport: (id: string, includeHeaderValues: boolean) => Promise<string | null>;
  }

  let { view, onsave, ontest, onremove, onexport }: Props = $props();

  interface InitialCustom {
    name: string;
    method: string;
    requestUrl: string;
    body: string;
    fileFormField: string;
    link: string;
    thumbnail: string;
    deletion: string;
    errorMessage: string;
    headers: HeaderRow[];
  }

  function initial(source: DestinationView): InitialCustom {
    return {
      name: source.name,
      method: source.method ?? 'POST',
      requestUrl: source.requestUrl ?? '',
      body: bodyLabel(source.body),
      fileFormField: source.fileFormField ?? 'file',
      link: pathDisplay(source.link),
      thumbnail: pathDisplay(source.thumbnail),
      deletion: pathDisplay(source.deletion),
      errorMessage: pathDisplay(source.errorMessage),
      headers: source.headers.map((header) => ({
        name: header.name,
        value: '',
        saved: header.saved,
      })),
    };
  }

  const start = untrack(() => initial(view));
  let name = $state(start.name);
  let method = $state(start.method);
  let requestUrl = $state(start.requestUrl);
  let body = $state(start.body);
  let fileFormField = $state(start.fileFormField);
  let link = $state(start.link);
  let thumbnail = $state(start.thumbnail);
  let deletion = $state(start.deletion);
  let errorMessage = $state(start.errorMessage);
  let headers = $state<HeaderRow[]>(start.headers);
  let includeHeaderValues = $state(false);
  let error = $state('');
  let saving = $state(false);
  let removing = $state(false);
  let exporting = $state(false);
  let testState = $state<'idle' | 'testing' | 'success' | 'failed'>('idle');
  let testMessage = $state<string | null>(null);

  function addHeader(): void {
    headers = [...headers, { name: '', value: '', saved: false }];
  }

  async function save(): Promise<void> {
    if (requestUrl.trim() === '') {
      error = 'Enter the request URL.';
      return;
    }
    if (bodyValue(body) === 'MultipartFormData' && fileFormField.trim() === '') {
      error = 'Enter the file form field.';
      return;
    }
    if (pathValue(link) === '') {
      error = 'Enter the link from the response.';
      return;
    }
    error = '';
    saving = true;
    try {
      const updated = await onsave({
        id: view.id,
        kind: 'custom',
        name: name.trim(),
        method,
        requestUrl: requestUrl.trim(),
        body: bodyValue(body),
        fileFormField: fileFormField.trim(),
        link: pathValue(link),
        thumbnail: pathValue(thumbnail) === '' ? null : pathValue(thumbnail),
        deletion: pathValue(deletion) === '' ? null : pathValue(deletion),
        errorMessage: pathValue(errorMessage) === '' ? null : pathValue(errorMessage),
        headers: headers.map((header) => ({
          name: header.name,
          value: header.value === '' ? null : header.value,
        })),
        importedFromSxcu: view.importedFromSxcu,
      });
      name = updated.name;
      headers = updated.headers.map((header) => ({
        name: header.name,
        value: '',
        saved: header.saved,
      }));
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

  async function exportUploader(): Promise<void> {
    error = '';
    exporting = true;
    try {
      await onexport(view.id, includeHeaderValues);
    } catch (failure) {
      error = errorText(failure);
    } finally {
      exporting = false;
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

<div class="destination-form" role="group" aria-label="Custom uploader">
  <div class="fields">
    <label class="field">
      <span class="label">Name</span>
      <input type="text" bind:value={name} oninput={() => (error = '')} />
    </label>
    <label class="field">
      <span class="label">Request method</span>
      <select bind:value={method} onchange={() => (error = '')}>
        {#each METHOD_OPTIONS as option}
          <option value={option}>{option}</option>
        {/each}
      </select>
    </label>
    <label class="field wide">
      <span class="label">Request URL</span>
      <input
        type="text"
        class="mono"
        spellcheck="false"
        bind:value={requestUrl}
        oninput={() => (error = '')}
      />
    </label>
    <label class="field">
      <span class="label">Body</span>
      <select bind:value={body} onchange={() => (error = '')}>
        {#each BODY_OPTIONS as option}
          <option value={option}>{option}</option>
        {/each}
      </select>
    </label>
    <label class="field">
      <span class="label">File form field</span>
      <input
        type="text"
        class="mono"
        spellcheck="false"
        bind:value={fileFormField}
        oninput={() => (error = '')}
      />
    </label>
    <label class="field">
      <span class="label">Link from response</span>
      <input
        type="text"
        class="mono"
        placeholder="$.data.url"
        spellcheck="false"
        bind:value={link}
        oninput={() => (error = '')}
      />
    </label>
    <label class="field">
      <span class="label">Thumbnail from response</span>
      <input
        type="text"
        class="mono"
        placeholder="$.data.thumbnail"
        spellcheck="false"
        bind:value={thumbnail}
        oninput={() => (error = '')}
      />
    </label>
    <label class="field">
      <span class="label">Deletion link from response</span>
      <input
        type="text"
        class="mono"
        placeholder="$.data.delete_url"
        spellcheck="false"
        bind:value={deletion}
        oninput={() => (error = '')}
      />
    </label>
    <label class="field">
      <span class="label">Error message from response</span>
      <input
        type="text"
        class="mono"
        placeholder="$.error.message"
        spellcheck="false"
        bind:value={errorMessage}
        oninput={() => (error = '')}
      />
    </label>
  </div>

  <div class="headers">
    <span class="label">Headers</span>
    {#each headers as header, index (index)}
      <div class="header-row">
        <input
          type="text"
          aria-label="Header name"
          class="mono"
          spellcheck="false"
          bind:value={header.name}
          oninput={() => (error = '')}
        />
        <SecretInput
          label="Header value"
          bind:value={header.value}
          saved={header.saved}
          oninput={() => (error = '')}
        />
      </div>
    {/each}
    <button type="button" class="add-header" onclick={addHeader}>
      <Icon name="plus" size={12} />
      Add header
    </button>
    <label class="include">
      <input type="checkbox" bind:checked={includeHeaderValues} />
      Include header values
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
      variant="secondary"
      height={34}
      onclick={exportUploader}
      disabled={exporting}
      title={exporting ? 'Exporting…' : undefined}
    >
      Export .sxcu
    </Button>
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

  input[type='text'],
  select {
    height: 34px;
    padding: 0 10px;
    border: 1px solid var(--stroke);
    border-bottom-color: var(--strokeHi);
    border-radius: 4px;
    background: var(--layer);
    color: var(--text);
    font-size: 12px;
  }

  select {
    font-size: 13px;
  }

  .mono {
    font-family: var(--type-mono);
  }

  .headers {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .header-row {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }

  .header-row > input {
    flex: 1 1 160px;
    min-width: 0;
  }

  .header-row > :global(label) {
    flex: 3 1 260px;
  }

  .add-header {
    align-self: flex-start;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 30px;
    padding: 0 8px;
    border: none;
    border-radius: 4px;
    background: transparent;
    color: var(--accent);
    font-size: 12px;
    cursor: pointer;
  }

  .include {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 12px;
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
