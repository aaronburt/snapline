export interface Workflow {
  id: string;
  name: string;
  chord: string;
  enabled: boolean;
  steps: string[];
  isDefault: boolean;
}

export const workflows: Workflow[] = [
  {
    id: 'f1',
    name: 'Screenshot to link',
    chord: 'PrtSc',
    enabled: true,
    steps: ['Capture region', 'Copy image', 'Upload to Imgur', 'Copy link'],
    isDefault: true,
  },
  {
    id: 'f2',
    name: 'Annotate and share',
    chord: 'Ctrl+Shift+A',
    enabled: true,
    steps: ['Capture region', 'Open editor', 'Upload', 'Copy as Markdown'],
    isDefault: false,
  },
  {
    id: 'f3',
    name: 'Bug report clip',
    chord: 'Ctrl+Alt+R',
    enabled: true,
    steps: ['Record region', 'Convert to GIF', 'Upload to S3', 'Copy link'],
    isDefault: false,
  },
  {
    id: 'f4',
    name: 'Grab text',
    chord: 'Ctrl+Alt+T',
    enabled: false,
    steps: ['Capture region', 'Extract text (OCR)', 'Copy text'],
    isDefault: false,
  },
  {
    id: 'f5',
    name: 'Save quietly',
    chord: 'Ctrl+Alt+PrtSc',
    enabled: false,
    steps: ['Capture screen', 'Save to folder'],
    isDefault: false,
  },
];

export const defaultWorkflowTooltip =
  'The default workflow runs on PrtSc and from the tray';

export function setWorkflowEnabled(
  list: Workflow[],
  id: string,
  enabled: boolean,
): Workflow[] {
  return list.map((workflow) =>
    workflow.id === id && !workflow.isDefault ? { ...workflow, enabled } : workflow,
  );
}

export function displayName(workflow: Workflow): string {
  return workflow.isDefault ? `${workflow.name} (default)` : workflow.name;
}
