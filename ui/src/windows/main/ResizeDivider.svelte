<script lang="ts">
  let { label, value, min, max, direction = 1, onresize }: { label: string; value: number; min: number; max: number; direction?: number; onresize: (value: number) => void } = $props();

  function start(event: PointerEvent) {
    if (event.button !== 0) return;
    const startX = event.clientX;
    const startValue = value;
    const move = (next: PointerEvent) => onresize(Math.min(max, Math.max(min, startValue + (next.clientX - startX) * direction)));
    const stop = () => {
      window.removeEventListener('pointermove', move);
      window.removeEventListener('pointerup', stop);
      window.removeEventListener('pointercancel', stop);
    };
    window.addEventListener('pointermove', move);
    window.addEventListener('pointerup', stop);
    window.addEventListener('pointercancel', stop);
    event.preventDefault();
  }

  function key(event: KeyboardEvent) {
    if (event.key !== 'ArrowLeft' && event.key !== 'ArrowRight') return;
    event.preventDefault();
    onresize(Math.min(max, Math.max(min, value + (event.key === 'ArrowRight' ? 10 : -10) * direction)));
  }
</script>

<div class="divider" role="slider" tabindex="0" aria-label={label} aria-orientation="horizontal" aria-valuemin={min} aria-valuemax={max} aria-valuenow={value} onpointerdown={start} onkeydown={key}></div>

<style>
  .divider { flex: 0 0 6px; cursor: col-resize; touch-action: none; }
  .divider:hover, .divider:focus-visible { background: var(--stroke); outline: none; }
</style>
