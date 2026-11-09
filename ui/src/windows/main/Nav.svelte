<script lang="ts">
  import Icon from '../../components/Icon.svelte';
  import NavItem from '../../components/NavItem.svelte';
  import { captureCount } from './history/data';

  interface Props {
    page: string;
  }

  let { page }: Props = $props();

  const section = $derived(
    page.startsWith('workflow')
      ? 'workflows'
      : page === 'qr' || page === 'combine' || page === 'convert'
        ? 'tools'
        : page,
  );

  function newCapture(): void {
    void 0;
  }
</script>

<nav aria-label="Main">
  <button type="button" class="newcapture" onclick={newCapture}>
    <Icon name="region" size={16} strokeWidth={1.8} />
    <span class="newlabel">New capture</span>
    <span class="chord">PrtSc</span>
  </button>

  <NavItem
    label="History"
    icon="history"
    count={captureCount}
    active={section === 'history'}
    href="#/history"
  />
  <NavItem label="Workflows" icon="workflows" active={section === 'workflows'} href="#/workflows" />
  <NavItem label="Tools" icon="tools" active={section === 'tools'} href="#/tools" />

  <div class="spacer"></div>

  <a
    class="hv destination"
    href="#/settings/uploads"
    aria-label="Default destination: Save locally only. Change in settings"
  >
    <Icon name="cloud" size={16} />
    Save locally only
  </a>

  <NavItem label="Settings" icon="gear" href="#/settings/general" />
</nav>

<style>
  nav {
    flex: 1 1 220px;
    max-width: 100%;
    padding: 4px 8px 16px;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .newcapture {
    height: 40px;
    margin: 0 4px 10px;
    border: 1px solid var(--accent);
    background: var(--accent);
    color: var(--onAccent);
    border-radius: 6px;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 0 12px;
    font-size: 13px;
    font-weight: 600;
    cursor: pointer;
  }

  .newlabel {
    flex: 1;
    text-align: left;
  }

  .chord {
    font-size: 11px;
    font-weight: 500;
    opacity: 0.85;
  }

  .spacer {
    flex: 1;
    min-height: 24px;
  }

  .destination {
    margin: 0 4px 8px;
    min-height: 44px;
    padding: 0 12px;
    border-radius: 6px;
    background: var(--subtle);
    border: 1px solid var(--stroke);
    display: flex;
    align-items: center;
    gap: 10px;
    color: var(--text);
    font-size: 13px;
    font-weight: 500;
  }
</style>
