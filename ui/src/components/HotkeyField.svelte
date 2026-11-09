<script lang="ts">
  interface Props {
    value?: string;
    label: string;
    buttonLabel?: string;
    clash?: string;
    onchange?: (value: string) => void;
  }

  let {
    value = $bindable(''),
    label,
    buttonLabel = 'Change',
    clash,
    onchange,
  }: Props = $props();

  let capturing = $state(false);
  let message = $state('');

  function begin() {
    capturing = true;
    message = '';
  }

  function keyName(key: string) {
    if (key === 'PrintScreen') {
      return 'PrtSc';
    }
    if (key === ' ') {
      return 'Space';
    }
    if (key.length === 1) {
      return key.toUpperCase();
    }
    return key;
  }

  function handle(event: KeyboardEvent) {
    if (!capturing) {
      return;
    }
    if (event.key === 'Escape') {
      event.preventDefault();
      capturing = false;
      message = '';
      return;
    }
    if (event.key === 'Backspace') {
      event.preventDefault();
      value = '';
      capturing = false;
      message = '';
      onchange?.('');
      return;
    }
    if (['Control', 'Alt', 'Shift', 'Meta'].includes(event.key)) {
      return;
    }
    const name = keyName(event.key);
    if (!name) {
      return;
    }
    const hasModifier = event.ctrlKey || event.altKey || event.shiftKey || event.metaKey;
    const allowedAlone =
      name === 'PrtSc' || name === 'Pause' || /^F([1-9]|1[0-9]|2[0-4])$/.test(name);
    if (!hasModifier && !allowedAlone) {
      return;
    }
    if (event.metaKey && !event.shiftKey) {
      event.preventDefault();
      message = 'Windows uses this shortcut.';
      return;
    }
    event.preventDefault();
    const chord: string[] = [];
    if (event.ctrlKey) {
      chord.push('Ctrl');
    }
    if (event.altKey) {
      chord.push('Alt');
    }
    if (event.shiftKey) {
      chord.push('Shift');
    }
    if (event.metaKey) {
      chord.push('Win');
    }
    chord.push(name);
    value = chord.join('+');
    capturing = false;
    message = '';
    onchange?.(value);
  }
</script>

<svelte:window onkeydown={handle} />

<div class="hotkey">
  <div class="field">
    <input
      type="text"
      readonly
      aria-label={label}
      aria-live="polite"
      value={capturing ? 'Press keys…' : value}
    />
    <button type="button" class="hv" onclick={begin}>{buttonLabel}</button>
  </div>
  {#if message}
    <span class="message" aria-live="polite">{message}</span>
  {:else if clash}
    <span class="message" aria-live="polite">{clash}</span>
  {/if}
</div>

<style>
  .hotkey {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .field {
    display: flex;
    gap: 6px;
  }

  input {
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
    font-family: var(--type-mono);
  }

  button {
    height: 34px;
    padding: 0 12px;
    border: 1px solid var(--stroke);
    border-bottom-color: var(--strokeHi);
    background: var(--layer);
    color: var(--text);
    border-radius: 4px;
    font-size: 13px;
    flex: none;
  }

  .message {
    font-size: 12px;
    color: var(--err);
  }
</style>
