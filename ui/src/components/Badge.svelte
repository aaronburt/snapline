<script lang="ts">
  import Icon from './Icon.svelte';
  import type { IconName } from '../lib/icons';

  type Status = 'ok' | 'local' | 'failed' | 'uploading';

  interface Props {
    status: Status;
    label?: string;
    long?: boolean;
  }

  let { status, label, long = false }: Props = $props();

  const states: Record<Status, { short: string; long: string; icon: IconName }> = {
    ok: { short: 'Link', long: 'Uploaded', icon: 'link' },
    local: { short: 'Local', long: 'Not uploaded', icon: 'folder' },
    failed: { short: 'Failed', long: 'Upload failed', icon: 'alert' },
    uploading: { short: 'Uploading', long: 'Uploading', icon: 'upload' },
  };

  const shown = $derived(label ?? (long ? states[status].long : states[status].short));
</script>

<span class="badge {status}">
  <Icon name={states[status].icon} size={11} strokeWidth={2.2} />
  {shown}
</span>

<style>
  .badge {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 1px 6px;
    border-radius: 10px;
    font-size: 11px;
    white-space: nowrap;
  }

  .ok {
    color: var(--ok);
    background: var(--okSoft);
  }

  .local {
    color: var(--text2);
    background: var(--subtle);
  }

  .failed {
    color: var(--err);
    background: var(--errSoft);
  }

  .uploading {
    color: var(--accent);
    background: var(--accentSoft);
  }
</style>
