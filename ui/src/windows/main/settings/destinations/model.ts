export const METHOD_OPTIONS = ['POST', 'PUT', 'PATCH'];

export const BODY_OPTIONS = ['Multipart form data', 'Binary', 'Form URL-encoded', 'JSON'];

const BODY_LABELS: Record<string, string> = {
  MultipartFormData: 'Multipart form data',
  Binary: 'Binary',
  FormURLEncoded: 'Form URL-encoded',
  JSON: 'JSON',
};

const BODY_VALUES: Record<string, string> = {
  'Multipart form data': 'MultipartFormData',
  Binary: 'Binary',
  'Form URL-encoded': 'FormURLEncoded',
  JSON: 'JSON',
};

export function bodyLabel(value: string | null | undefined): string {
  return BODY_LABELS[value ?? ''] ?? 'Multipart form data';
}

export function bodyValue(label: string): string {
  return BODY_VALUES[label] ?? 'MultipartFormData';
}

export function pathDisplay(value: string | null | undefined): string {
  if (!value) {
    return '';
  }
  if (value.startsWith('{json:') && value.endsWith('}')) {
    return value.slice(6, -1);
  }
  return value;
}

export function pathValue(display: string): string {
  const value = display.trim();
  if (value === '' || value.startsWith('{')) {
    return value;
  }
  if (value.startsWith('$')) {
    return `{json:${value}}`;
  }
  return value;
}

export function destinationStatusText(configured: boolean): string {
  return configured ? 'Configured' : 'Not set up';
}

export function destinationActionText(action: string): string {
  if (action === 'configure') {
    return 'Configure';
  }
  if (action === 'edit') {
    return 'Edit';
  }
  return 'Set up';
}

export function destinationIcon(kind: string): 'image' | 'bucket' | 'code' | 'server' | 'cloud' {
  if (kind === 's3') {
    return 'bucket';
  }
  if (kind === 'gist') {
    return 'code';
  }
  if (kind === 'sftp') {
    return 'server';
  }
  if (kind === 'custom') {
    return 'cloud';
  }
  return 'image';
}

export function errorText(error: unknown): string {
  const candidate = error as { message?: unknown } | null;
  if (candidate && typeof candidate.message === 'string') {
    return candidate.message;
  }
  return 'The destination could not be saved.';
}
