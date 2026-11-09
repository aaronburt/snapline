<script lang="ts">
  import Icon from './Icon.svelte';
  import type { StepItem } from './types';

  interface Props {
    items?: StepItem[];
    onchange?: (items: StepItem[]) => void;
  }

  let { items = $bindable([]), onchange }: Props = $props();
  let dragIndex = $state<number | null>(null);

  function move(from: number, to: number) {
    if (to < 0 || to >= items.length || from === to) {
      return;
    }
    const next = [...items];
    const [item] = next.splice(from, 1);
    next.splice(to, 0, item);
    items = next;
    onchange?.(next);
  }

  function remove(index: number) {
    const next = items.filter((_, position) => position !== index);
    items = next;
    onchange?.(next);
  }

  function flip(index: number, checked: boolean) {
    const next = items.map((item, position) => (position === index ? { ...item, checked } : item));
    items = next;
    onchange?.(next);
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
</script>

<div class="steps" role="list">
  {#each items as item, index (item.id)}
    <div
      class="step"
      role="listitem"
      ondragover={(event) => event.preventDefault()}
      ondrop={(event) => drop(event, index)}
    >
      <button
        type="button"
        class="handle"
        aria-label={`Reorder: ${item.label}`}
        draggable="true"
        ondragstart={(event) => startDrag(event, index)}
        onkeydown={(event) => keydown(event, index)}
      >
        <Icon name="drag" size={14} strokeWidth={3} />
      </button>
      <span class="number">{index + 1}</span>
      {#if item.removable}
        <div class="body">
          <button
            type="button"
            class="remove hv"
            aria-label={`Remove: ${item.label}`}
            onclick={() => remove(index)}
          >
            <Icon name="close" size={14} />
          </button>
          <Icon name={item.icon} size={18} />
          <span class="label">{item.label}</span>
        </div>
      {:else}
        <label class="body">
          <input
            type="checkbox"
            checked={item.checked}
            onchange={(event) => flip(index, event.currentTarget.checked)}
          />
          <Icon name={item.icon} size={18} />
          <span class="label" class:off={!item.checked}>{item.label}</span>
        </label>
      {/if}
      {#if item.note}
        <span class="note">{item.note}</span>
      {/if}
    </div>
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
    padding: 0 16px 0 6px;
  }

  .step + .step {
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

  .body {
    flex: 1 1 200px;
    min-width: 0;
    min-height: 44px;
    display: flex;
    align-items: center;
    gap: 12px;
    padding-left: 8px;
    cursor: pointer;
  }

  input {
    width: 18px;
    height: 18px;
    margin: 0;
    accent-color: var(--accent);
    flex: none;
  }

  .label {
    font-size: 13px;
  }

  .label.off {
    color: var(--text3);
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
  }

  .note {
    font-size: 12px;
    color: var(--text3);
    white-space: nowrap;
  }
</style>
