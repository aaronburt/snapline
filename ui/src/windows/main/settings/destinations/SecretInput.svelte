<script lang="ts">
  import Icon from '../../../../components/Icon.svelte';

  interface Props {
    label: string;
    value?: string;
    saved?: boolean;
    oninput?: (value: string) => void;
  }

  let { label, value = $bindable(''), saved = false, oninput }: Props = $props();
  let visible = $state(false);
  const id = $props.id();
</script>

<label class="field" for={id}>
  <span class="label">{label}</span>
  <span class="row">
    <input
      {id}
      type={visible ? 'text' : 'password'}
      aria-label={label}
      placeholder={saved ? 'Saved' : ''}
      spellcheck="false"
      autocomplete="off"
      {value}
      oninput={(event) => {
        value = event.currentTarget.value;
        oninput?.(value);
      }}
    />
    <button
      type="button"
      class="hv"
      aria-label={visible ? `Hide ${label}` : `Show ${label}`}
      aria-pressed={visible}
      onclick={() => (visible = !visible)}
    >
      <Icon name={visible ? 'eyeOff' : 'eye'} size={16} />
    </button>
  </span>
</label>

<style>
  .field {
    display: flex;
    flex-direction: column;
    gap: 6px;
    min-width: 0;
  }

  .label {
    font-size: 12px;
    color: var(--text2);
  }

  .row {
    display: flex;
    gap: 6px;
    min-width: 0;
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
    width: 34px;
    height: 34px;
    flex: none;
    display: flex;
    align-items: center;
    justify-content: center;
    border: 1px solid var(--stroke);
    border-radius: 4px;
    background: var(--layer);
    color: var(--text2);
  }

  button:not(:disabled) {
    cursor: pointer;
  }
</style>
