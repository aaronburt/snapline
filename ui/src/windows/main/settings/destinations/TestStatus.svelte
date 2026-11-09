<script lang="ts">
  interface Props {
    state: 'idle' | 'testing' | 'success' | 'failed';
    message?: string | null;
  }

  let { state, message = null }: Props = $props();

  const text = $derived(
    state === 'testing'
      ? 'Testing…'
      : state === 'success'
        ? 'Test upload succeeded · link copied'
        : state === 'failed'
          ? `Test failed: ${message ?? 'The test upload failed.'}`
          : 'Not tested yet',
  );
</script>

<span class="pill" class:ok={state === 'success'} class:err={state === 'failed'} role="status">
  {text}
</span>

<style>
  .pill {
    display: inline-flex;
    align-items: center;
    padding: 3px 10px;
    border-radius: 12px;
    background: var(--subtle);
    color: var(--text3);
    font-size: 12px;
  }

  .ok {
    background: var(--okSoft);
    color: var(--ok);
  }

  .err {
    background: color-mix(in srgb, var(--err) 12%, transparent);
    color: var(--err);
  }
</style>
