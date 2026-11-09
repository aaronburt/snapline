import type { IconName } from '../../../lib/icons';
import type { HotkeysState } from '../../../lib/ipc/hotkeys';
import type { CaptureStepKindDto } from '../../../lib/ipc/bindings/CaptureStepKindDto';
import type { DestinationOption } from '../../../lib/ipc/bindings/DestinationOption';
import type { LinkFormatDto } from '../../../lib/ipc/bindings/LinkFormatDto';
import type { RecordStepKindDto } from '../../../lib/ipc/bindings/RecordStepKindDto';
import type { StepDto } from '../../../lib/ipc/bindings/StepDto';
import type { ViolationDto } from '../../../lib/ipc/bindings/ViolationDto';
import type { WorkflowInfo } from '../../../lib/ipc/bindings/WorkflowInfo';
import type { WorkflowInput } from '../../../lib/ipc/bindings/WorkflowInput';

export const defaultWorkflowTooltip = 'The default workflow runs on PrtSc and from the tray';

export const NAME_ERROR = 'The name must be 1 to 60 characters.';
export const FIRST_STEP_ERROR = 'The first step must be a Capture or Record step.';
export const COPY_LINK_ERROR = 'Copy link requires an earlier Upload step.';
export const COPY_TEXT_ERROR = 'Copy text requires an earlier Extract text step.';
export const CONVERT_GIF_ERROR = 'Convert to GIF requires an earlier Record step.';

export const captureKindLabels: Record<CaptureStepKindDto, string> = {
  region: 'region',
  window: 'window',
  screen: 'screen',
  scrolling: 'scrolling',
  last_region: 'last region',
};

export const recordKindLabels: Record<RecordStepKindDto, string> = {
  video: 'video',
  gif: 'GIF',
  window: 'window',
};

export const linkFormatLabels: Record<LinkFormatDto, string> = {
  direct: 'Direct link',
  markdown: 'Markdown',
  html: 'HTML <img>',
  bbcode: 'BBCode',
  page: 'Page link',
};

export interface StepChoice {
  value: string;
  label: string;
}

export const stepChoices: StepChoice[] = [
  { value: 'capture', label: 'Capture' },
  { value: 'record', label: 'Record' },
  { value: 'copy_image', label: 'Copy image' },
  { value: 'annotate', label: 'Annotate' },
  { value: 'save', label: 'Save' },
  { value: 'upload', label: 'Upload' },
  { value: 'copy_link', label: 'Copy link' },
  { value: 'extract_text', label: 'Extract text' },
  { value: 'copy_text', label: 'Copy text' },
  { value: 'convert_to_gif', label: 'Convert to GIF' },
  { value: 'notify', label: 'Notify' },
  { value: 'play_sound', label: 'Play sound' },
];

export function displayName(workflow: WorkflowInfo): string {
  return workflow.isDefault ? `${workflow.name} (default)` : workflow.name;
}

export function stepLabel(step: StepDto): string {
  switch (step.step) {
    case 'capture':
      return `Capture ${captureKindLabels[step.kind]}`;
    case 'record':
      return `Record ${recordKindLabels[step.kind]}`;
    case 'copy_image':
      return 'Copy image';
    case 'annotate':
      return 'Annotate';
    case 'save':
      return 'Save';
    case 'upload':
      return 'Upload';
    case 'copy_link':
      return 'Copy link';
    case 'extract_text':
      return 'Extract text';
    case 'copy_text':
      return 'Copy text';
    case 'convert_to_gif':
      return 'Convert to GIF';
    case 'notify':
      return 'Notify';
    case 'play_sound':
      return 'Play sound';
  }
}

export function stepIcon(step: StepDto): IconName {
  switch (step.step) {
    case 'capture':
      return step.kind === 'scrolling'
        ? 'scroll'
        : step.kind === 'last_region'
          ? 'stop'
          : step.kind;
    case 'record':
      return step.kind === 'gif' ? 'gifMark' : step.kind === 'window' ? 'window' : 'record';
    case 'copy_image':
      return 'copy';
    case 'annotate':
      return 'edit';
    case 'save':
      return 'save';
    case 'upload':
      return 'upload';
    case 'copy_link':
      return 'link';
    case 'extract_text':
      return 'ocr';
    case 'copy_text':
      return 'text';
    case 'convert_to_gif':
      return 'gif';
    case 'notify':
      return 'bell';
    case 'play_sound':
      return 'speaker';
  }
}

export function stepKey(step: StepDto, index: number): string {
  return `${step.step}-${index}`;
}

export function newStep(value: string): StepDto | null {
  switch (value) {
    case 'capture':
      return { step: 'capture', kind: 'region' };
    case 'record':
      return { step: 'record', kind: 'video' };
    case 'copy_image':
      return { step: 'copy_image' };
    case 'annotate':
      return { step: 'annotate' };
    case 'save':
      return { step: 'save' };
    case 'upload':
      return { step: 'upload', dest: null };
    case 'copy_link':
      return { step: 'copy_link', format: 'direct' };
    case 'extract_text':
      return { step: 'extract_text' };
    case 'copy_text':
      return { step: 'copy_text' };
    case 'convert_to_gif':
      return { step: 'convert_to_gif' };
    case 'notify':
      return { step: 'notify' };
    case 'play_sound':
      return { step: 'play_sound' };
    default:
      return null;
  }
}

export function workflowOwner(workflow: WorkflowInfo): string {
  return workflow.isDefault ? 'capture_region' : `workflow:${workflow.id}`;
}

export function chordFor(workflow: WorkflowInfo, live: HotkeysState): string {
  const owner = workflowOwner(workflow);
  const binding = live.bindings.find((entry) => entry.owner === owner);
  if (binding) {
    return binding.chord;
  }
  const blocked = live.blocked.find((entry) => entry.owner === owner);
  if (blocked) {
    return blocked.chord;
  }
  return workflow.hotkey ?? '—';
}

export function isBlocked(workflow: WorkflowInfo, live: HotkeysState): boolean {
  const owner = workflowOwner(workflow);
  return live.blocked.some((entry) => entry.owner === owner);
}

export function clashLine(workflow: WorkflowInfo, live: HotkeysState): string | null {
  const owner = workflowOwner(workflow);
  const entry = live.blocked.find((row) => row.owner === owner);
  if (!entry || entry.reason !== 'internal') {
    return null;
  }
  const other = live.blocked.find(
    (row) => row.owner !== owner && row.chord === entry.chord && row.reason === 'internal',
  );
  if (!other) {
    return null;
  }
  return `Off because ${entry.chord} is also used by ${other.label}.`;
}

export function ownerLabel(owner: string, live: HotkeysState): string {
  const known = [...live.bindings, ...live.blocked].find((entry) => entry.owner === owner);
  if (known) {
    return known.label;
  }
  if (owner.startsWith('workflow:')) {
    return `Workflow ${owner.slice('workflow:'.length)}`;
  }
  return owner;
}

export function formatOwners(labels: string[], fallback: string): string {
  if (labels.length === 0) {
    return fallback;
  }
  if (labels.length === 1) {
    return labels[0];
  }
  if (labels.length === 2) {
    return `${labels[0]} and ${labels[1]}`;
  }
  return `${labels.slice(0, -1).join(', ')} and ${labels[labels.length - 1]}`;
}

export function workflowInput(workflow: WorkflowInfo): WorkflowInput {
  return {
    name: workflow.name,
    hotkey: workflow.hotkey,
    enabled: workflow.enabled,
    steps: workflow.steps,
  };
}

function isCaptureOrRecord(step: StepDto): boolean {
  return step.step === 'capture' || step.step === 'record';
}

export function validateWorkflow(
  name: string,
  steps: StepDto[],
  isDefault: boolean,
): ViolationDto[] {
  const violations: ViolationDto[] = [];
  const characters = Array.from(name).length;
  if (name.trim().length === 0 || characters < 1 || characters > 60) {
    violations.push({ step: null, message: NAME_ERROR });
  }
  if (!isDefault) {
    if (steps.length === 0) {
      violations.push({ step: null, message: FIRST_STEP_ERROR });
    } else if (!isCaptureOrRecord(steps[0])) {
      violations.push({ step: 0, message: FIRST_STEP_ERROR });
    }
  }
  let uploaded = false;
  let extracted = false;
  let recorded = false;
  steps.forEach((step, index) => {
    switch (step.step) {
      case 'upload':
        uploaded = true;
        break;
      case 'extract_text':
        extracted = true;
        break;
      case 'record':
        recorded = true;
        break;
      case 'copy_link':
        if (!uploaded) {
          violations.push({ step: index, message: COPY_LINK_ERROR });
        }
        break;
      case 'copy_text':
        if (!extracted) {
          violations.push({ step: index, message: COPY_TEXT_ERROR });
        }
        break;
      case 'convert_to_gif':
        if (!recorded) {
          violations.push({ step: index, message: CONVERT_GIF_ERROR });
        }
        break;
      default:
        break;
    }
  });
  return violations;
}

export function destinationOptions(destinations: DestinationOption[]): StepChoice[] {
  return [
    { value: '', label: 'Default destination' },
    ...destinations.map((destination) => ({
      value: destination.id,
      label: destination.name,
    })),
  ];
}
