<script lang="ts">
  import Button from '../../../components/Button.svelte';
  import Icon from '../../../components/Icon.svelte';
  import { navigate } from '../../../router';
  import { defaultWorkflowTooltip, displayName, setWorkflowEnabled, workflows } from './data';

  let flows = $state(workflows.map((workflow) => ({ ...workflow })));
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

  <div class="list">
    {#each flows as workflow (workflow.id)}
      <article class="card">
        <div class="main">
          <div class="nameRow">
            <span class="name" class:off={!workflow.enabled}>{displayName(workflow)}</span>
            <span class="chord">{workflow.chord}</span>
          </div>
          <div class="steps">
            {#each workflow.steps as step, index (step)}
              {#if index > 0}
                <span class="chevron"><Icon name="chevronRight" size={12} strokeWidth={2} /></span>
              {/if}
              <span class="step">{step}</span>
            {/each}
          </div>
        </div>
        <div class="side">
          <Button
            variant="secondary"
            height={32}
            onclick={() => {
              navigate(`/workflows/${workflow.id}`);
            }}>Edit</Button
          >
          <label
            class="toggle"
            title={workflow.isDefault ? defaultWorkflowTooltip : undefined}
          >
            {workflow.enabled ? 'On' : 'Off'}
            <input
              type="checkbox"
              class="tg"
              aria-label={`Enable ${displayName(workflow)}`}
              title={workflow.isDefault ? defaultWorkflowTooltip : undefined}
              checked={workflow.enabled}
              disabled={workflow.isDefault}
              onchange={(event) => {
                flows = setWorkflowEnabled(
                  flows,
                  workflow.id,
                  (event.currentTarget as HTMLInputElement).checked,
                );
              }}
            />
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
