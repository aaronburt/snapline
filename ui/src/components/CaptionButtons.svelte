<script lang="ts">
  import { onMount } from 'svelte';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import { captionIcons } from '../lib/icons';

  let maximised = $state(false);

  function minimise() {
    void getCurrentWindow().minimize();
  }

  function toggle() {
    void getCurrentWindow().toggleMaximize();
  }

  function close() {
    void getCurrentWindow().close();
  }

  onMount(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    const appWindow = getCurrentWindow();
    void appWindow
      .isMaximized()
      .then((value) => {
        if (!disposed) {
          maximised = value;
        }
      })
      .catch(() => undefined);
    void appWindow
      .onResized(() => {
        void appWindow
          .isMaximized()
          .then((value) => {
            if (!disposed) {
              maximised = value;
            }
          })
          .catch(() => undefined);
      })
      .then((stop) => {
        if (disposed) {
          stop();
        } else {
          unlisten = stop;
        }
      })
      .catch(() => undefined);
    return () => {
      disposed = true;
      unlisten?.();
    };
  });
</script>

<div class="captions">
  <button type="button" class="hv" aria-label="Minimise" onclick={minimise}>
    <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true">
      <path d={captionIcons.minimise} stroke="currentColor" stroke-width="1"></path>
    </svg>
  </button>
  <button type="button" class="hv" aria-label={maximised ? 'Restore' : 'Maximise'} onclick={toggle}>
    <svg
      width="10"
      height="10"
      viewBox="0 0 10 10"
      fill="none"
      stroke="currentColor"
      stroke-width="1"
      aria-hidden="true"
    >
      <path d={maximised ? captionIcons.restore : captionIcons.maximise}></path>
    </svg>
  </button>
  <button type="button" class="hv" aria-label="Close" onclick={close}>
    <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true">
      <path d={captionIcons.close} stroke="currentColor" stroke-width="1"></path>
    </svg>
  </button>
</div>

<style>
  .captions {
    display: flex;
    flex: none;
  }

  button {
    width: 46px;
    height: 44px;
    border: none;
    background: transparent;
    color: var(--text);
    display: flex;
    align-items: center;
    justify-content: center;
  }
</style>
