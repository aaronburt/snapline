<script lang="ts">
  import Icon from '../../../components/Icon.svelte';
  import type { CaptureStepKindDto } from '../../../lib/ipc/bindings/CaptureStepKindDto';
  import type { DestinationOption } from '../../../lib/ipc/bindings/DestinationOption';
  import type { LinkFormatDto } from '../../../lib/ipc/bindings/LinkFormatDto';
  import type { StepDto } from '../../../lib/ipc/bindings/StepDto';
  import type { ViolationDto } from '../../../lib/ipc/bindings/ViolationDto';
  import {
    captureKindLabels,
    destinationOptions,
    linkFormatLabels,
    stepIcon,
    stepKey,
    stepLabel,
  } from './data';

  interface Props {
    steps: StepDto[];
    destinations: DestinationOption[];
    errors: ViolationDto[];
    onchange?: (steps: StepDto[]) => void;
  }

  let { steps, destinations, errors, onchange }: Props = $props();
  let dragIndex = $state<number | null>(null);

  const captureOptions = Object.entries(captureKindLabels) as [CaptureStepKindDto, string][];
  const formatOptions = Object.entries(linkFormatLabels) as [LinkFormatDto, string][];

  function move(from: number, to: number) {
    if (to < 0 || to >= steps.length || from === to) {
      return;
    }
    const next = [...steps];
    const [item] = next.splice(from, 1);
    next.splice(to, 0, item);
    onchange?.(next);
  }

  function remove(index: number) {
    onchange?.(steps.filter((_, position) => position !== index));
  }

  function update(index: number, step: StepDto) {
    onchange?.(steps.map((item, position) => (position === index ? step : item)));
  }

  function startDrag(event: DragEvent, index: number) {
    dragIndex = index;
    event.dataTransfer?.setData('text/plain', String(index));
    if (event.dataTransfer) {
      event.dataTransfer.effectAllowed = 'move';
    }
  }

  function drop(event: DragEvent, index: number) {
    event.preventDefault();
    const from = dragIndex ?? Number(event.dataTransfer?.getData('text/plain'));
    dragIndex = null;
    if (Number.isInteger(from)) {
      move(from, index);
    }
  }

  function keydown(event: KeyboardEvent, index: number) {
    if (!event.altKey) {
      return;
    }
    if (event.key === 'ArrowUp') {
      event.preventDefault();
      move(index, index - 1);
    } else if (event.key === 'ArrowDown') {
      event.preventDefault();
      move(index, index + 1);
    }
  }

  function errorsAt(index: number): ViolationDto[] {
    return errors.filter((violation) => violation.step === index);
  }

  function captureKind(event: Event): CaptureStepKindDto {
    return (event.currentTarget as HTMLSelectElement).value as CaptureStepKindDto;
  }

  function linkFormat(event: Event): LinkFormatDto {
    return (event.currentTarget as HTMLSelectElement).value as LinkFormatDto;
  }

  function destination(event: Event): string | null {
    const value = (event.currentTarget as HTMLSelectElement).value;
    return value === '' ? null : value;
  }
</script>

<div class="steps" role="list">
  {#each steps as step, index (stepKey(step, index))}
    <div
      class="step"
      role="listitem"
      ondragover={(event) => event.preventDefault()}
      ondrop={(event) => drop(event, index)}
    >
      <button
        type="button"
        class="handle"
        aria-label={`Reorder: ${stepLabel(step)}`}
        draggable="true"
        ondragstart={(event) => startDrag(event, index)}
        onkeydown={(event) => keydown(event, index)}
      >
        <Icon name="drag" size={14} strokeWidth={3} />
      </button>
      <span class="number">{index + 1}</span>
      <Icon name={stepIcon(step)} size={18} />
      <span class="label">{stepLabel(step)}</span>
      {#if step.step === 'capture'}
        <select
          aria-label="Capture kind"
          value={step.kind}
          onchange={(event) => update(index, { step: 'capture', kind: captureKind(event) })}
        >
          {#each captureOptions as [value, label]}
            <option {value}>Capture {label}</option>
          {/each}
        </select>
      {:else if step.step === 'upload'}
        <select
          aria-label="Upload destination"
          value={step.dest ?? ''}
          onchange={(event) => update(index, { step: 'upload', dest: destination(event) })}
        >
          {#each destinationOptions(destinations) as option}
            <option value={option.value}>{option.label}</option>
          {/each}
        </select>
      {:else if step.step === 'copy_link'}
        <select
          aria-label="Copy link format"
          value={step.format}
          onchange={(event) => update(index, { step: 'copy_link', format: linkFormat(event) })}
        >
          {#each formatOptions as [value, label]}
            <option {value}>{label}</option>
          {/each}
        </select>
      {/if}
      <button
        type="button"
        class="remove hv"
        aria-label={`Remove: ${stepLabel(step)}`}
        onclick={() => remove(index)}
      >
        <Icon name="close" size={14} />
      </button>
    </div>
    {#each errorsAt(index) as violation}
      <p class="error">{violation.message}</p>
    {/each}
  {/each}
</div>

<style>
  .steps {
    background: var(--card);
    border: 1px solid var(--stroke);
    border-radius: 6px;
    overflow: hidden;
    margin-bottom: 4px;
  }

  .step {
    display: flex;
    align-items: center;
    gap: 8px;
    min-height: 52px;
    padding: 0 12px 0 6px;
  }

  .step + .step,
  .error + .step {
    border-top: 1px solid var(--stroke);
  }

  .handle {
    width: 28px;
    height: 40px;
    border: none;
    background: transparent;
    color: var(--text3);
    cursor: grab;
    display: flex;
    align-items: center;
    justify-content: center;
    border-radius: 4px;
    flex: none;
  }

  .number {
    width: 18px;
    font-size: 12px;
    color: var(--text3);
    text-align: right;
    flex: none;
  }

  .label {
    flex: 1 1 120px;
    min-width: 0;
    font-size: 13px;
  }

  select {
    height: 30px;
    min-width: 132px;
    padding: 0 8px;
    border: 1px solid var(--stroke);
    border-bottom-color: var(--strokeHi);
    border-radius: 4px;
    background: var(--layer);
    color: var(--text);
    font-size: 12px;
    flex: none;
  }

  .remove {
    width: 28px;
    height: 28px;
    padding: 0;
    border: none;
    background: transparent;
    color: var(--text3);
    display: flex;
    align-items: center;
    justify-content: center;
    border-radius: 4px;
    flex: none;
  }

  .error {
    margin: 0;
    padding: 4px 16px 8px 60px;
    font-size: 12px;
    color: var(--err);
  }
</style>
