<script lang="ts">
  interface Props {
    options: string[];
    value?: string;
    label?: string;
    labelledby?: string;
    onchange?: (value: string) => void;
  }

  let {
    options,
    value = $bindable(''),
    label,
    labelledby,
    onchange,
  }: Props = $props();
</script>

<div class="segmented" role="radiogroup" aria-label={label} aria-labelledby={labelledby}>
  {#each options as option}
    <button
      type="button"
      role="radio"
      aria-checked={value === option}
      class:selected={value === option}
      onclick={() => {
        value = option;
        onchange?.(option);
      }}
    >
      {option}
    </button>
  {/each}
</div>

<style>
  .segmented {
    display: flex;
    padding: 2px;
    background: var(--subtle);
    border: 1px solid var(--stroke);
    border-radius: 6px;
  }

  button {
    height: 30px;
    min-width: 52px;
    padding: 0 12px;
    border: none;
    border-radius: 4px;
    background: transparent;
    color: var(--text2);
    font-size: 12px;
    cursor: pointer;
  }

  button.selected {
    background: var(--card);
    color: var(--text);
    font-weight: 600;
    box-shadow: 0 1px 2px var(--shadow);
  }
</style>
