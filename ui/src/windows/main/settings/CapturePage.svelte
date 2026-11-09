<script lang="ts">
  import { onMount } from 'svelte';
  import Card from '../../../components/Card.svelte';
  import Icon from '../../../components/Icon.svelte';
  import ToggleRow from '../../../components/ToggleRow.svelte';
  import SelectRow from '../../../components/SelectRow.svelte';
  import SegmentedRow from '../../../components/SegmentedRow.svelte';
  import RangeRow from '../../../components/RangeRow.svelte';
  import FieldRow from '../../../components/FieldRow.svelte';
  import SettingsPage from './SettingsPage.svelte';
  import SettingsSection from './SettingsSection.svelte';
  import {
    CAPTURE_DELAY_OPTIONS,
    GIF_FRAME_RATE_OPTIONS,
    GIF_WIDTH_OPTIONS,
    MAGNIFIER_ZOOM_OPTIONS,
    MAX_SCROLL_OPTIONS,
    MICROPHONE_OPTIONS,
    MONITOR_OPTIONS,
    QUALITY_OPTIONS,
    RECORDING_FRAME_RATE_OPTIONS,
    VIDEO_FORMAT_OPTIONS,
    loadSettings,
    settings,
    updateCapture,
  } from './settings';

  interface Props {
    filter?: string;
  }

  let { filter = '' }: Props = $props();

  const rows = $derived($settings.capture);

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

  const cursorVisible = $derived(
    matches('Include mouse cursor', 'Draw the pointer into screenshots'),
  );
  const delayVisible = $derived(
    matches('Capture delay', 'Wait before capturing, so you can open a menu first'),
  );
  const shutterVisible = $derived(
    matches('Shutter sound', 'Play a sound when a capture is taken'),
  );
  const shadowVisible = $derived(
    matches(
      'Keep window shadow and rounded corners',
      'Window captures keep the Windows 11 shadow on a transparent background',
    ),
  );
  const showScreenshots = $derived(
    matches('Screenshots') || cursorVisible || delayVisible || shutterVisible || shadowVisible,
  );

  const freezeVisible = $derived(matches('Freeze the screen while selecting'));
  const magnifierVisible = $derived(
    matches('Magnifier', 'Pixel-accurate zoom next to the cursor'),
  );
  const zoomVisible = $derived(matches('Magnifier zoom'));
  const snapVisible = $derived(
    matches('Snap to windows and controls', 'Hover to pick a whole window, panel or button, then click'),
  );
  const sizeVisible = $derived(
    matches('Show size and position', 'Width, height and coordinates while dragging'),
  );
  const crosshairVisible = $derived(
    matches('Full-screen crosshair', 'Guide lines across every monitor'),
  );
  const showRegion = $derived(
    matches('Region selection') ||
      freezeVisible ||
      magnifierVisible ||
      zoomVisible ||
      snapVisible ||
      sizeVisible ||
      crosshairVisible,
  );

  const monitorsVisible = $derived(
    matches('Monitors', 'Choose what a full-screen capture (Ctrl+PrtSc) includes.') ||
      matches('Full-screen capture uses'),
  );

  const videoFormatVisible = $derived(
    matches('Video format', 'H.264 plays everywhere; H.265 is smaller'),
  );
  const frameRateVisible = $derived(matches('Frame rate', 'Frames per second'));
  const qualityVisible = $derived(matches('Quality'));
  const systemAudioVisible = $derived(matches('Record system audio', 'Sound playing on this PC'));
  const microphoneVisible = $derived(matches('Record microphone'));
  const microphoneDeviceVisible = $derived(matches('Microphone device'));
  const borderVisible = $derived(
    matches('Show recording border', 'Outline the recorded area (not included in the video)'),
  );
  const ffmpegVisible = $derived(
    matches('FFmpeg location', 'Used to encode video. Found automatically.'),
  );
  const showRecording = $derived(
    matches('Screen recording') ||
      videoFormatVisible ||
      frameRateVisible ||
      qualityVisible ||
      systemAudioVisible ||
      microphoneVisible ||
      microphoneDeviceVisible ||
      borderVisible ||
      ffmpegVisible,
  );

  const gifFrameRateVisible = $derived(
    matches('Frame rate', 'Lower frame rates make much smaller GIFs'),
  );
  const gifWidthVisible = $derived(matches('Maximum width'));
  const paletteVisible = $derived(
    matches('Optimise colour palette', 'Smaller files, slightly slower to encode'),
  );
  const showGif = $derived(
    matches('GIF') || gifFrameRateVisible || gifWidthVisible || paletteVisible,
  );

  const scrollDelayVisible = $derived(
    matches('Delay between scrolls', 'Raise this for pages that load content as you scroll'),
  );
  const maxScrollsVisible = $derived(matches('Stop after this many scrolls'));
  const stickyVisible = $derived(
    matches('Remove sticky headers', 'Hide headers and footers that repeat on every scroll'),
  );
  const showScrolling = $derived(
    matches('Scrolling capture') || scrollDelayVisible || maxScrollsVisible || stickyVisible,
  );

  const monitors = $derived([
    {
      n: '1',
      res: '1920 × 1080 · primary',
      w: 216,
      h: 122,
      captured: rows.monitorMode !== 'Monitor under cursor',
      cursor: false,
    },
    {
      n: '2',
      res: '2560 × 1440',
      w: 288,
      h: 162,
      captured: rows.monitorMode !== 'Primary monitor',
      cursor: rows.monitorMode === 'Monitor under cursor',
    },
  ]);

  const monitorCaption = $derived(
    rows.monitorMode === 'All monitors'
      ? 'All monitors are captured as one 4480 × 1440 image'
      : rows.monitorMode === 'Primary monitor'
        ? 'Monitor 1 (primary) is captured'
        : 'The monitor with the cursor is captured',
  );

  const monitorLabel = $derived(
    `Monitor 1, 1920 by 1080, primary; Monitor 2, 2560 by 1440. Highlighted: ${rows.monitorMode}`,
  );
</script>

<SettingsPage title="Capture">
  {#if showScreenshots}
    <SettingsSection title="Screenshots">
      {#if cursorVisible}
        <ToggleRow
          icon="cursor"
          title="Include mouse cursor"
          description="Draw the pointer into screenshots"
          checked={rows.includeCursor}
          onchange={(value) => updateCapture('includeCursor', value)}
        />
      {/if}
      {#if delayVisible}
        <SegmentedRow
          icon="timer"
          title="Capture delay"
          description="Wait before capturing, so you can open a menu first"
          options={CAPTURE_DELAY_OPTIONS}
          value={rows.captureDelay}
          onchange={(value) => updateCapture('captureDelay', value)}
        />
      {/if}
      {#if shutterVisible}
        <ToggleRow
          icon="speaker"
          title="Shutter sound"
          description="Play a sound when a capture is taken"
          checked={rows.shutterSound}
          onchange={(value) => updateCapture('shutterSound', value)}
        />
      {/if}
      {#if shadowVisible}
        <ToggleRow
          icon="shadow"
          title="Keep window shadow and rounded corners"
          description="Window captures keep the Windows 11 shadow on a transparent background"
          checked={rows.windowShadow}
          onchange={(value) => updateCapture('windowShadow', value)}
        />
      {/if}
    </SettingsSection>
  {/if}

  {#if showRegion}
    <SettingsSection title="Region selection">
      {#if freezeVisible}
        <ToggleRow
          icon="freeze"
          title="Freeze the screen while selecting"
          checked={rows.freezeScreen}
          onchange={(value) => updateCapture('freezeScreen', value)}
        />
      {/if}
      {#if magnifierVisible}
        <ToggleRow
          icon="searchPlus"
          title="Magnifier"
          description="Pixel-accurate zoom next to the cursor"
          checked={rows.magnifier}
          onchange={(value) => updateCapture('magnifier', value)}
        />
      {/if}
      {#if zoomVisible}
        <SegmentedRow
          icon="search"
          title="Magnifier zoom"
          options={MAGNIFIER_ZOOM_OPTIONS}
          value={rows.magnifierZoom}
          onchange={(value) => updateCapture('magnifierZoom', value)}
        />
      {/if}
      {#if snapVisible}
        <ToggleRow
          icon="snap"
          title="Snap to windows and controls"
          description="Hover to pick a whole window, panel or button, then click"
          checked={rows.snapWindows}
          onchange={(value) => updateCapture('snapWindows', value)}
        />
      {/if}
      {#if sizeVisible}
        <ToggleRow
          icon="ruler"
          title="Show size and position"
          description="Width, height and coordinates while dragging"
          checked={rows.showSize}
          onchange={(value) => updateCapture('showSize', value)}
        />
      {/if}
      {#if crosshairVisible}
        <ToggleRow
          icon="cross"
          title="Full-screen crosshair"
          description="Guide lines across every monitor"
          checked={rows.crosshair}
          onchange={(value) => updateCapture('crosshair', value)}
        />
      {/if}
    </SettingsSection>
  {/if}

  {#if monitorsVisible}
    <SettingsSection
      title="Monitors"
      description="Choose what a full-screen capture (Ctrl+PrtSc) includes."
    >
      <Card>
        <div class="monitor-card">
          <div class="monitor-layout" role="img" aria-label={monitorLabel}>
            {#each monitors as monitor}
              <div
                class="monitor-box"
                class:on={monitor.captured}
                style:width="{monitor.w}px"
                style:height="{monitor.h}px"
              >
                <span class="monitor-number">{monitor.n}</span>
                <span class="monitor-resolution">{monitor.res}</span>
                {#if monitor.cursor}
                  <span class="monitor-cursor">
                    <Icon name="cursor" size={18} />
                  </span>
                {/if}
              </div>
            {/each}
          </div>
          <div class="monitor-caption">{monitorCaption}</div>
        </div>
      </Card>
      <SelectRow
        icon="screen"
        title="Full-screen capture uses"
        options={MONITOR_OPTIONS}
        value={rows.monitorMode}
        onchange={(value) => updateCapture('monitorMode', value)}
      />
    </SettingsSection>
  {/if}

  {#if showRecording}
    <SettingsSection title="Screen recording">
      {#if videoFormatVisible}
        <SelectRow
          icon="film"
          title="Video format"
          description="H.264 plays everywhere; H.265 is smaller"
          options={VIDEO_FORMAT_OPTIONS}
          value={rows.videoFormat}
          onchange={(value) => updateCapture('videoFormat', value)}
        />
      {/if}
      {#if frameRateVisible}
        <SegmentedRow
          icon="fps"
          title="Frame rate"
          description="Frames per second"
          options={RECORDING_FRAME_RATE_OPTIONS}
          value={rows.frameRate}
          onchange={(value) => updateCapture('frameRate', value)}
        />
      {/if}
      {#if qualityVisible}
        <SelectRow
          icon="gauge"
          title="Quality"
          options={QUALITY_OPTIONS}
          value={rows.quality}
          onchange={(value) => updateCapture('quality', value)}
        />
      {/if}
      {#if systemAudioVisible}
        <ToggleRow
          icon="audio"
          title="Record system audio"
          description="Sound playing on this PC"
          checked={rows.systemAudio}
          onchange={(value) => updateCapture('systemAudio', value)}
        />
      {/if}
      {#if microphoneVisible}
        <ToggleRow
          icon="mic"
          title="Record microphone"
          checked={rows.microphone}
          onchange={(value) => updateCapture('microphone', value)}
        />
      {/if}
      {#if microphoneDeviceVisible}
        <SelectRow
          icon="mic"
          title="Microphone device"
          options={MICROPHONE_OPTIONS}
          value={rows.microphoneDevice}
          onchange={(value) => updateCapture('microphoneDevice', value)}
        />
      {/if}
      {#if borderVisible}
        <ToggleRow
          icon="border"
          title="Show recording border"
          description="Outline the recorded area (not included in the video)"
          checked={rows.recordingBorder}
          onchange={(value) => updateCapture('recordingBorder', value)}
        />
      {/if}
      {#if ffmpegVisible}
        <FieldRow
          icon="code"
          title="FFmpeg location"
          description="Used to encode video. Found automatically."
          mono
          button="Browse…"
          value={rows.ffmpegPath}
          onchange={(value) => updateCapture('ffmpegPath', value)}
        />
      {/if}
    </SettingsSection>
  {/if}

  {#if showGif}
    <SettingsSection title="GIF">
      {#if gifFrameRateVisible}
        <SegmentedRow
          icon="fps"
          title="Frame rate"
          description="Lower frame rates make much smaller GIFs"
          options={GIF_FRAME_RATE_OPTIONS}
          value={rows.gifFrameRate}
          onchange={(value) => updateCapture('gifFrameRate', value)}
        />
      {/if}
      {#if gifWidthVisible}
        <SelectRow
          icon="width"
          title="Maximum width"
          options={GIF_WIDTH_OPTIONS}
          value={rows.gifWidth}
          onchange={(value) => updateCapture('gifWidth', value)}
        />
      {/if}
      {#if paletteVisible}
        <ToggleRow
          icon="palette"
          title="Optimise colour palette"
          description="Smaller files, slightly slower to encode"
          checked={rows.optimisePalette}
          onchange={(value) => updateCapture('optimisePalette', value)}
        />
      {/if}
    </SettingsSection>
  {/if}

  {#if showScrolling}
    <SettingsSection title="Scrolling capture">
      {#if scrollDelayVisible}
        <RangeRow
          icon="scroll"
          title="Delay between scrolls"
          description="Raise this for pages that load content as you scroll"
          min={100}
          max={1000}
          step={50}
          unit=" ms"
          value={rows.scrollDelay}
          onchange={(value) => updateCapture('scrollDelay', value)}
        />
      {/if}
      {#if maxScrollsVisible}
        <SelectRow
          icon="stop"
          title="Stop after this many scrolls"
          options={MAX_SCROLL_OPTIONS}
          value={rows.maxScrolls}
          onchange={(value) => updateCapture('maxScrolls', value)}
        />
      {/if}
      {#if stickyVisible}
        <ToggleRow
          icon="sticky"
          title="Remove sticky headers"
          description="Hide headers and footers that repeat on every scroll"
          checked={rows.removeStickyHeaders}
          onchange={(value) => updateCapture('removeStickyHeaders', value)}
        />
      {/if}
    </SettingsSection>
  {/if}
</SettingsPage>

<style>
  .monitor-card {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 14px;
    padding: 24px 20px 18px;
  }

  .monitor-layout {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-end;
    justify-content: center;
    gap: 10px;
  }

  .monitor-box {
    position: relative;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 2px;
    border-radius: 6px;
    background: var(--subtle);
    border: 1px solid var(--strokeHi);
    color: var(--text3);
  }

  .monitor-box.on {
    background: var(--accentSoft);
    border: 2px solid var(--accent);
    color: var(--text);
  }

  .monitor-number {
    font-size: 22px;
    font-weight: 600;
  }

  .monitor-resolution {
    font-size: 11px;
    opacity: 0.85;
  }

  .monitor-cursor {
    position: absolute;
    right: 22px;
    bottom: 18px;
  }

  .monitor-caption {
    font-size: 12px;
    color: var(--text3);
  }
</style>
