<script lang="ts">
  import Button from '../../../components/Button.svelte';
  import HotkeyField from '../../../components/HotkeyField.svelte';
  import Icon from '../../../components/Icon.svelte';
  import type { DestinationOption } from '../../../lib/ipc/bindings/DestinationOption';
  import type { PreviewDto } from '../../../lib/ipc/bindings/PreviewDto';
  import type { StepDto } from '../../../lib/ipc/bindings/StepDto';
  import type { WorkflowInfo } from '../../../lib/ipc/bindings/WorkflowInfo';
  import type { WorkflowInput } from '../../../lib/ipc/bindings/WorkflowInput';
  import { get as getHotkeys } from '../../../lib/ipc/hotkeys';
  import type { HotkeysState } from '../../../lib/ipc/hotkeys';
  import { preview as previewHotkey } from '../../../lib/ipc/hotkeys';
  import {
    create,
    duplicate as duplicateWorkflow,
    get,
    list,
    remove,
    update,
  } from '../../../lib/ipc/workflow';
  import { currentRoute, navigate } from '../../../router';
  import {
    FIRST_STEP_ERROR,
    NAME_ERROR,
    defaultWorkflowTooltip,
    formatOwners,
    newStep,
    ownerLabel,
    stepChoices,
    validateWorkflow,
  } from './data';
  import StepEditor from './StepEditor.svelte';

  const routeId = $derived($currentRoute.params.id ?? null);
  const nameId = $props.id();

  let name = $state('');
  let hotkey = $state('');
  let enabled = $state(true);
  let steps = $state<StepDto[]>([]);
  let destinations = $state<DestinationOption[]>([]);
  let live = $state<HotkeysState>({ bindings: [], blocked: [], paused: false });
  let isDefault = $state(false);
  let loading = $state(true);
  let addValue = $state('');
  let message = $state('');
  let clash = $state('');

  let loadToken = 0;
  let previewToken = 0;

  const title = $derived(routeId === null ? 'New workflow' : name || 'New workflow');
  const owner = $derived(isDefault ? 'capture_region' : `workflow:${routeId ?? 'new'}`);
  const issues = $derived(validateWorkflow(name, steps, isDefault));
  const nameError = $derived(issues.find((issue) => issue.message === NAME_ERROR)?.message ?? '');
  const listError = $derived(
    issues.find((issue) => issue.step === null && issue.message === FIRST_STEP_ERROR)?.message ??
      '',
  );

  $effect(() => {
    void load(routeId);
  });

  async function load(target: string | null): Promise<void> {
    const token = ++loadToken;
    loading = true;
    isDefault = target === 'default';
    try {
      const [result, hotkeys, workflow] = await Promise.all([
        list(),
        getHotkeys(),
        target === null ? Promise.resolve<WorkflowInfo | null>(null) : get(target),
      ]);
      if (token !== loadToken) {
        return;
      }
      destinations = result.destinations;
      live = hotkeys;
      if (workflow) {
        name = workflow.name;
        hotkey = workflow.hotkey ?? '';
        enabled = workflow.enabled;
        steps = workflow.steps;
        isDefault = workflow.isDefault;
      } else {
        name = '';
        hotkey = '';
        enabled = true;
        steps = [];
        isDefault = false;
      }
      addValue = '';
      clash = '';
      message = '';
      if (hotkey) {
        void checkHotkey(hotkey);
      }
    } catch (cause) {
      if (token === loadToken) {
        message = messageOf(cause);
      }
    } finally {
      if (token === loadToken) {
        loading = false;
      }
    }
  }

  async function checkHotkey(value: string): Promise<void> {
    const token = ++previewToken;
    if (value.trim().length === 0) {
      clash = '';
      return;
    }
    try {
      const preview = await previewHotkey(owner, value);
      if (token === previewToken) {
        clash = previewText(preview);
      }
    } catch (cause) {
      if (token === previewToken) {
        clash = messageOf(cause);
      }
    }
  }

  function previewText(preview: PreviewDto): string {
    if (preview === 'free') {
      return '';
    }
    if (preview === 'external') {
      return 'Off: another app is using this shortcut.';
    }
    const labels = preview.internal.owners
      .filter((entry) => entry !== owner)
      .map((entry) => ownerLabel(entry, live));
    return labels.length === 0 ? '' : `Off: also used by ${formatOwners(labels, '')}.`;
  }

  function changeHotkey(value: string): void {
    hotkey = value;
    void checkHotkey(value);
  }

  function addStep(event: Event): void {
    const select = event.currentTarget as HTMLSelectElement;
    const step = newStep(select.value);
    if (step) {
      steps = [...steps, step];
    }
    addValue = '';
    select.value = '';
  }

  function input(): WorkflowInput {
    return {
      name,
      hotkey: hotkey.trim().length === 0 ? null : hotkey,
      enabled,
      steps,
    };
  }

  async function save(): Promise<void> {
    if (issues.length > 0) {
      message = issues[0].message;
      return;
    }
    try {
      if (routeId === null) {
        await create(input());
      } else {
        await update(routeId, input());
      }
      navigate('/workflows');
    } catch (cause) {
      message = messageOf(cause);
    }
  }

  async function duplicateFlow(): Promise<void> {
    if (routeId === null) {
      return;
    }
    try {
      const copy = await duplicateWorkflow(routeId);
      navigate(`/workflows/${copy.id}`);
    } catch (cause) {
      message = messageOf(cause);
    }
  }

  async function deleteFlow(): Promise<void> {
    if (routeId === null || isDefault) {
      return;
    }
    if (!window.confirm(`Delete ${name}?`)) {
      return;
    }
    try {
      await remove(routeId);
      navigate('/workflows');
    } catch (cause) {
      message = messageOf(cause);
    }
  }

  function messageOf(cause: unknown): string {
    if (typeof cause === 'object' && cause !== null && 'message' in cause) {
      const value = (cause as { message?: unknown }).message;
      if (typeof value === 'string') {
        return value;
      }
    }
    return 'The workflow could not be saved.';
  }
</script>

<main class="page">
  <div class="pagehead">
    <div class="titles">
      <h1>{title}</h1>
      <div class="sub">Steps run in order after the hotkey is pressed.</div>
    </div>
  </div>

  {#if !loading}
    <div class="form">
      <div class="row">
        <Icon name="workflows" size={20} />
        <div class="text">
          <label class="title" for={nameId}>Name</label>
        </div>
        <div class="field">
          <input
            id={nameId}
            type="text"
            bind:value={name}
            disabled={isDefault}
            spellcheck="false"
          />
        </div>
      </div>
      {#if nameError}
        <p class="error">{nameError}</p>
      {/if}

      {#if !isDefault}
        <div class="row">
          <Icon name="hotkeys" size={20} />
          <div class="text">
            <span class="title">Hotkey</span>
          </div>
          <div class="field">
            <HotkeyField label="Hotkey" bind:value={hotkey} {clash} onchange={changeHotkey} />
          </div>
        </div>
      {/if}

      <label class="row hv" title={isDefault ? defaultWorkflowTooltip : undefined}>
        <Icon name="power" size={20} />
        <span class="text">
          <span class="title">Enabled</span>
        </span>
        <span class="state">{enabled ? 'On' : 'Off'}</span>
        <input
          type="checkbox"
          class="tg"
          aria-label="Enabled"
          title={isDefault ? defaultWorkflowTooltip : undefined}
          bind:checked={enabled}
          disabled={isDefault}
        />
      </label>

      <div class="section">
        <h2>Steps</h2>
        <StepEditor
          {steps}
          {destinations}
          errors={issues}
          onchange={(next) => {
            steps = next;
          }}
        />
        {#if listError}
          <p class="error">{listError}</p>
        {/if}
        <div class="add">
          <select aria-label="Add step" bind:value={addValue} onchange={addStep}>
            <option value="" disabled>Add step</option>
            {#each stepChoices as choice}
              <option value={choice.value}>{choice.label}</option>
            {/each}
          </select>
        </div>
      </div>
    </div>

    {#if message}
      <p class="message" role="alert">{message}</p>
    {/if}

    <div class="footer">
      <Button variant="accent" height={34} onclick={save}>Save</Button>
      <Button
        variant="secondary"
        height={34}
        onclick={() => {
          navigate('/workflows');
        }}>Cancel</Button
      >
      <span class="spacer"></span>
      {#if routeId !== null}
        <Button variant="secondary" height={34} onclick={duplicateFlow}>Duplicate</Button>
        <Button variant="danger-text" height={34} disabled={isDefault} onclick={deleteFlow}
          >Delete</Button
        >
      {/if}
    </div>
  {/if}
</main>

<style>
  .page {
    flex: 999 1 460px;
    min-width: 0;
    padding: 20px 24px 32px;
    display: flex;
    flex-direction: column;
    gap: 20px;
    overflow-y: auto;
  }

  .pagehead {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-end;
    justify-content: space-between;
    gap: 12px;
  }

  .titles {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  h1 {
    margin: 0;
    font-size: 28px;
    line-height: 36px;
    font-weight: 600;
  }

  .sub {
    font-size: 12px;
    color: var(--text3);
  }

  .form {
    display: flex;
    flex-direction: column;
    gap: 8px;
    max-width: 1000px;
  }

  .section {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin-top: 12px;
  }

  h2 {
    margin: 0 0 4px;
    font-size: 14px;
    font-weight: 600;
  }

  .row {
    min-height: 68px;
    padding: 12px 20px;
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 12px 16px;
    background: var(--card);
    border: 1px solid var(--stroke);
    border-radius: 6px;
  }

  label.row {
    cursor: pointer;
  }

  .text {
    flex: 1 1 220px;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .title {
    font-size: 14px;
  }

  .field {
    display: flex;
    gap: 6px;
    flex: 1 1 320px;
    max-width: 460px;
  }

  .field input {
    flex: 1;
    min-width: 0;
    height: 34px;
    padding: 0 10px;
    border: 1px solid var(--stroke);
    border-bottom-color: var(--strokeHi);
    border-radius: 4px;
    background: var(--layer);
    color: var(--text);
    font-size: 12px;
  }

  .field input:disabled {
    color: var(--text3);
    cursor: default;
  }

  .state {
    font-size: 13px;
    color: var(--text2);
    width: 24px;
    text-align: right;
  }

  .add {
    display: flex;
  }

  .add select {
    height: 32px;
    min-width: 160px;
    padding: 0 10px;
    border: 1px solid var(--stroke);
    border-bottom-color: var(--strokeHi);
    border-radius: 4px;
    background: var(--layer);
    color: var(--text);
    font-size: 13px;
  }

  .error,
  .message {
    margin: 0;
    font-size: 12px;
    color: var(--err);
  }

  .footer {
    display: flex;
    align-items: center;
    gap: 8px;
    padding-top: 16px;
    border-top: 1px solid var(--stroke);
    max-width: 1000px;
  }

  .spacer {
    flex: 1;
  }
</style>
