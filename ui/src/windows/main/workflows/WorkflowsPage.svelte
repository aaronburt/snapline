<script lang="ts">
  import { onMount } from 'svelte';
  import Button from '../../../components/Button.svelte';
  import EmptyState from '../../../components/EmptyState.svelte';
  import Icon from '../../../components/Icon.svelte';
  import { navigate } from '../../../router';
  import type { WorkflowInfo } from '../../../lib/ipc/bindings/WorkflowInfo';
  import { get as getHotkeys } from '../../../lib/ipc/hotkeys';
  import type { HotkeysState } from '../../../lib/ipc/hotkeys';
  import { list, update } from '../../../lib/ipc/workflow';
  import {
    chordFor,
    clashLine,
    defaultWorkflowTooltip,
    displayName,
    isBlocked,
    stepLabel,
    workflowInput,
  } from './data';

  let workflows = $state<WorkflowInfo[]>([]);
  let live = $state<HotkeysState>({ bindings: [], blocked: [], paused: false });
  let loaded = $state(false);
  let message = $state('');

  onMount(() => {
    void load();
  });

  async function load(): Promise<void> {
    try {
      const [result, hotkeys] = await Promise.all([list(), getHotkeys()]);
      workflows = result.workflows;
      live = hotkeys;
      message = '';
    } catch (cause) {
      message = messageOf(cause);
    } finally {
      loaded = true;
    }
  }

  async function toggle(workflow: WorkflowInfo, enabled: boolean): Promise<void> {
    if (workflow.isDefault) {
      return;
    }
    try {
      const updated = await update(workflow.id, { ...workflowInput(workflow), enabled });
      workflows = workflows.map((item) => (item.id === updated.id ? updated : item));
      live = await getHotkeys();
      message = '';
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
    return 'The workflows could not be loaded.';
  }
</script>

<main class="page">
  <div class="pagehead">
    <div class="titles">
      <h1>Workflows</h1>
      <div class="sub">A workflow is a hotkey plus the steps that run after the capture</div>
    </div>
    <Button
      variant="accent"
      height={34}
      icon="plus"
      onclick={() => {
        navigate('/workflows/new');
      }}>New workflow</Button
    >
  </div>

  {#if message}
    <p class="message" role="alert">{message}</p>
  {/if}

  {#if loaded && workflows.length === 0}
    <EmptyState title="No workflows yet" />
  {/if}

  <div class="list">
    {#each workflows as workflow (workflow.id)}
      {@const blocked = isBlocked(workflow, live)}
      {@const clash = clashLine(workflow, live)}
      <article class="card">
        <div class="main">
          <div class="nameRow">
            <span class="name" class:off={!workflow.enabled}>{displayName(workflow)}</span>
            <span class="chord" class:blocked>{chordFor(workflow, live)}</span>
          </div>
          <div class="steps">
            {#each workflow.steps as step, index (index)}
              {#if index > 0}
                <span class="chevron"><Icon name="chevronRight" size={12} strokeWidth={2} /></span>
              {/if}
              <span class="step">{stepLabel(step)}</span>
            {/each}
          </div>
          {#if clash}
            <p class="clash" role="status">{clash}</p>
          {/if}
        </div>
        <div class="side">
          <Button
            variant="secondary"
            height={32}
            onclick={() => {
              navigate(`/workflows/${workflow.id}`);
            }}>Edit</Button
          >
          <label class="toggle" title={workflow.isDefault ? defaultWorkflowTooltip : undefined}>
            {workflow.enabled ? 'On' : 'Off'}
            <input
              type="checkbox"
              class="tg"
              aria-label={`Enable ${displayName(workflow)}`}
              aria-describedby={workflow.isDefault ? `default-note-${workflow.id}` : undefined}
              title={workflow.isDefault ? defaultWorkflowTooltip : undefined}
              checked={workflow.enabled}
              disabled={workflow.isDefault}
              onchange={(event) => {
                void toggle(workflow, (event.currentTarget as HTMLInputElement).checked);
              }}
            />
            {#if workflow.isDefault}
              <span id={`default-note-${workflow.id}`} class="sr-only"
                >{defaultWorkflowTooltip}</span
              >
            {/if}
          </label>
        </div>
      </article>
    {/each}
  </div>
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

  .message,
  .clash {
    margin: 0;
    font-size: 12px;
    color: var(--err);
  }

  .list {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .card {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 12px 20px;
    padding: 16px 20px;
    background: var(--card);
    border: 1px solid var(--stroke);
    border-radius: 6px;
  }

  .main {
    flex: 1 1 420px;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .nameRow {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
  }

  .name {
    font-size: 14px;
    font-weight: 600;
  }

  .name.off {
    color: var(--text3);
  }

  .chord {
    font-size: 11px;
    padding: 2px 8px;
    border-radius: 4px;
    border: 1px solid var(--strokeHi);
    color: var(--text2);
    font-family: var(--type-mono);
  }

  .chord.blocked {
    color: var(--err);
    border-color: var(--err);
  }

  .steps {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
  }

  .chevron {
    display: flex;
    color: var(--text3);
  }

  .step {
    font-size: 12px;
    padding: 4px 10px;
    border-radius: 12px;
    background: var(--subtle);
    color: var(--text);
  }

  .side {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .toggle {
    display: flex;
    align-items: center;
    gap: 10px;
    min-height: 44px;
    padding: 0 4px;
    cursor: pointer;
    font-size: 13px;
    color: var(--text2);
  }

  .toggle:has(input:disabled) {
    cursor: default;
  }
</style>
