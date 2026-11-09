<script lang="ts">
  import { onMount } from 'svelte';
  import Icon from '../../../components/Icon.svelte';
  import HotkeyField from '../../../components/HotkeyField.svelte';
  import ToggleRow from '../../../components/ToggleRow.svelte';
  import SettingsPage from './SettingsPage.svelte';
  import SettingsSection from './SettingsSection.svelte';
  import { loadSettings, settings, updateHotkeys } from './settings';

  interface Props {
    filter?: string;
  }

  let { filter = '' }: Props = $props();

  const rows = $derived($settings.hotkeys);

  onMount(() => {
    void loadSettings();
  });

  function matches(title: string, description = ''): boolean {
    const needle = filter.trim().toLowerCase();
    return (
      needle === '' ||
      title.toLowerCase().includes(needle) ||
      description.toLowerCase().includes(needle)
    );
  }

  const regionVisible = $derived(matches('Capture region', 'Runs the default workflow'));
  const windowVisible = $derived(matches('Capture window'));
  const fullScreenVisible = $derived(matches('Capture full screen'));
  const scrollingVisible = $derived(matches('Scrolling capture'));
  const lastRegionVisible = $derived(matches('Last region again'));
  const showCapture = $derived(
    matches('Capture') ||
      regionVisible ||
      windowVisible ||
      fullScreenVisible ||
      scrollingVisible ||
      lastRegionVisible,
  );

  const videoVisible = $derived(matches('Record video'));
  const gifVisible = $derived(matches('Record GIF'));
  const showRecording = $derived(matches('Recording') || videoVisible || gifVisible);

  const textVisible = $derived(matches('Text capture (OCR)'));
  const colourVisible = $derived(matches('Colour picker'));
  const rulerVisible = $derived(matches('Screen ruler'));
  const uploadVisible = $derived(matches('Upload from clipboard'));
  const showTools = $derived(
    matches('Tools') || textVisible || colourVisible || rulerVisible || uploadVisible,
  );

  const overrideVisible = $derived(
    matches('Take over the PrtSc key', 'Stops Windows opening Snipping Tool when you press PrtSc'),
  );
  const clashVisible = $derived(
    matches('Warn about clashes', 'Tell me when another app already uses a hotkey'),
  );
  const showBehaviour = $derived(matches('Behaviour') || overrideVisible || clashVisible);
</script>

<SettingsPage
  title="Hotkeys"
  intro="If two actions share a chord, or another app owns it, neither is bound and you are told."
>
  {#if showCapture}
    <SettingsSection title="Capture">
      {#if regionVisible}
        <div class="row">
          <Icon name="region" size={20} />
          <div class="text">
            <span class="title">Capture region</span>
            <span class="description">Runs the default workflow</span>
          </div>
          <div class="control">
            <HotkeyField
              label="Capture region"
              value={rows.captureRegion}
              onchange={(value) => updateHotkeys('captureRegion', value)}
            />
          </div>
        </div>
      {/if}
      {#if windowVisible}
        <div class="row">
          <Icon name="window" size={20} />
          <div class="text">
            <span class="title">Capture window</span>
          </div>
          <div class="control">
            <HotkeyField
              label="Capture window"
              value={rows.captureWindow}
              onchange={(value) => updateHotkeys('captureWindow', value)}
            />
          </div>
        </div>
      {/if}
      {#if fullScreenVisible}
        <div class="row">
          <Icon name="screen" size={20} />
          <div class="text">
            <span class="title">Capture full screen</span>
          </div>
          <div class="control">
            <HotkeyField
              label="Capture full screen"
              value={rows.captureFullScreen}
              onchange={(value) => updateHotkeys('captureFullScreen', value)}
            />
          </div>
        </div>
      {/if}
      {#if scrollingVisible}
        <div class="row">
          <Icon name="scroll" size={20} />
          <div class="text">
            <span class="title">Scrolling capture</span>
          </div>
          <div class="control">
            <HotkeyField
              label="Scrolling capture"
              value={rows.scrollingCapture}
              onchange={(value) => updateHotkeys('scrollingCapture', value)}
            />
          </div>
        </div>
      {/if}
      {#if lastRegionVisible}
        <div class="row">
          <Icon name="stop" size={20} />
          <div class="text">
            <span class="title">Last region again</span>
          </div>
          <div class="control">
            <HotkeyField
              label="Last region again"
              value={rows.lastRegion}
              onchange={(value) => updateHotkeys('lastRegion', value)}
            />
          </div>
        </div>
      {/if}
    </SettingsSection>
  {/if}

  {#if showRecording}
    <SettingsSection title="Recording">
      {#if videoVisible}
        <div class="row">
          <Icon name="film" size={20} />
          <div class="text">
            <span class="title">Record video</span>
          </div>
          <div class="control">
            <HotkeyField
              label="Record video"
              value={rows.recordVideo}
              onchange={(value) => updateHotkeys('recordVideo', value)}
            />
          </div>
        </div>
      {/if}
      {#if gifVisible}
        <div class="row">
          <Icon name="gifMark" size={20} />
          <div class="text">
            <span class="title">Record GIF</span>
          </div>
          <div class="control">
            <HotkeyField
              label="Record GIF"
              value={rows.recordGif}
              onchange={(value) => updateHotkeys('recordGif', value)}
            />
          </div>
        </div>
      {/if}
    </SettingsSection>
  {/if}

  {#if showTools}
    <SettingsSection title="Tools">
      {#if textVisible}
        <div class="row">
          <Icon name="ocr" size={20} />
          <div class="text">
            <span class="title">Text capture (OCR)</span>
          </div>
          <div class="control">
            <HotkeyField
              label="Text capture (OCR)"
              value={rows.textCapture}
              onchange={(value) => updateHotkeys('textCapture', value)}
            />
          </div>
        </div>
      {/if}
      {#if colourVisible}
        <div class="row">
          <Icon name="palette" size={20} />
          <div class="text">
            <span class="title">Colour picker</span>
          </div>
          <div class="control">
            <HotkeyField
              label="Colour picker"
              value={rows.colourPicker}
              onchange={(value) => updateHotkeys('colourPicker', value)}
            />
          </div>
        </div>
      {/if}
      {#if rulerVisible}
        <div class="row">
          <Icon name="ruler" size={20} />
          <div class="text">
            <span class="title">Screen ruler</span>
          </div>
          <div class="control">
            <HotkeyField
              label="Screen ruler"
              value={rows.screenRuler}
              onchange={(value) => updateHotkeys('screenRuler', value)}
            />
          </div>
        </div>
      {/if}
      {#if uploadVisible}
        <div class="row">
          <Icon name="clip" size={20} />
          <div class="text">
            <span class="title">Upload from clipboard</span>
          </div>
          <div class="control">
            <HotkeyField
              label="Upload from clipboard"
              value={rows.uploadClipboard}
              onchange={(value) => updateHotkeys('uploadClipboard', value)}
            />
          </div>
        </div>
      {/if}
    </SettingsSection>
  {/if}

  {#if showBehaviour}
    <SettingsSection title="Behaviour">
      {#if overrideVisible}
        <ToggleRow
          icon="shield"
          title="Take over the PrtSc key"
          description="Stops Windows opening Snipping Tool when you press PrtSc"
          checked={rows.takeOverPrtSc}
          onchange={(value) => updateHotkeys('takeOverPrtSc', value)}
        />
      {/if}
      {#if clashVisible}
        <ToggleRow
          icon="alert"
          title="Warn about clashes"
          description="Tell me when another app already uses a hotkey"
          checked={rows.warnClashes}
          onchange={(value) => updateHotkeys('warnClashes', value)}
        />
      {/if}
    </SettingsSection>
  {/if}
</SettingsPage>

<style>
  .row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 12px 16px;
    min-height: 68px;
    padding: 12px 20px;
    background: var(--card);
    border: 1px solid var(--stroke);
    border-radius: 6px;
  }

  .text {
    display: flex;
    flex: 1 1 220px;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }

  .title {
    font-size: 14px;
  }

  .description {
    font-size: 12px;
    color: var(--text3);
  }

  .control {
    flex: 1 1 320px;
    max-width: 460px;
  }
</style>
