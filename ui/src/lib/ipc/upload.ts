import type { UploadReq } from './bindings/UploadReq';
import type { UploadResp } from './bindings/UploadResp';
import { call } from './core';

export async function copyLink(link: string): Promise<void> {
  const req: UploadReq = { kind: 'copyLink', data: { link } };
  const response = await call<UploadReq, UploadResp>('ipc_upload', req);
  if (response.kind !== 'linkCopied') {
    throw new Error('The link could not be copied.');
  }
}

export type DestinationKind = 'imgur' | 's3' | 'gist' | 'sftp' | 'custom';
export type DestinationStatus = 'configured' | 'not_set_up';
export type DestinationAction = 'configure' | 'set_up' | 'edit';
export type DestinationTestStatus = 'not_tested' | 'succeeded' | 'failed';

export interface DestinationHeader {
  name: string;
  saved: boolean;
}

export interface DestinationView {
  id: string;
  kind: DestinationKind;
  name: string;
  detail: string;
  status: DestinationStatus;
  action: DestinationAction;
  configured: boolean;
  clientIdSaved: boolean;
  endpoint: string | null;
  region: string | null;
  bucket: string | null;
  pathPrefix: string | null;
  publicUrlPattern: string | null;
  aclPublicRead: boolean;
  accessKeyIdSaved: boolean;
  secretAccessKeySaved: boolean;
  method: string | null;
  requestUrl: string | null;
  body: string | null;
  fileFormField: string | null;
  link: string | null;
  thumbnail: string | null;
  deletion: string | null;
  errorMessage: string | null;
  headers: DestinationHeader[];
  importedFromSxcu: boolean;
  host: string | null;
  port: number | null;
  username: string | null;
  authMethod: string | null;
  privateKeyPath: string | null;
  remoteFolder: string | null;
  passwordSaved: boolean;
  passphraseSaved: boolean;
  warnings: string[];
}

export interface DestinationHeaderInput {
  name: string;
  value?: string | null;
}

export interface DestinationInput {
  id?: string | null;
  kind: DestinationKind;
  name?: string | null;
  clientId?: string | null;
  endpoint?: string | null;
  region?: string | null;
  bucket?: string | null;
  accessKeyId?: string | null;
  secretAccessKey?: string | null;
  pathPrefix?: string | null;
  publicUrlPattern?: string | null;
  aclPublicRead?: boolean | null;
  method?: string | null;
  requestUrl?: string | null;
  body?: string | null;
  fileFormField?: string | null;
  link?: string | null;
  thumbnail?: string | null;
  deletion?: string | null;
  errorMessage?: string | null;
  headers: DestinationHeaderInput[];
  importedFromSxcu?: boolean | null;
  host?: string | null;
  port?: number | null;
  username?: string | null;
  authMethod?: string | null;
  privateKeyPath?: string | null;
  remoteFolder?: string | null;
  password?: string | null;
  passphrase?: string | null;
}

export interface DestinationTestOutcome {
  status: DestinationTestStatus;
  link: string | null;
  message: string | null;
}

export interface ImportedDestination {
  destination: DestinationView | null;
  warnings: string[];
}

export type DestinationReq =
  | { kind: 'list' }
  | { kind: 'add'; data: { kind: DestinationKind } }
  | { kind: 'save'; data: { input: DestinationInput } }
  | { kind: 'remove'; data: { id: string } }
  | { kind: 'test'; data: { id: string } }
  | { kind: 'importSxcu' }
  | { kind: 'exportSxcu'; data: { id: string; includeHeaderValues: boolean } };

export type DestinationResp =
  | { kind: 'destinations'; data: DestinationView[] }
  | { kind: 'destination'; data: DestinationView }
  | { kind: 'removed'; data: { id: string } }
  | { kind: 'test'; data: { outcome: DestinationTestOutcome } }
  | { kind: 'imported'; data: ImportedDestination }
  | { kind: 'exported'; data: { path: string | null } };

interface DestinationCall {
  kind: 'destination';
  data: DestinationReq;
}

interface DestinationReply {
  kind: 'destination';
  data: DestinationResp;
}

async function destination(req: DestinationReq): Promise<DestinationResp> {
  const reply = await call<DestinationCall, DestinationReply>('ipc_upload', {
    kind: 'destination',
    data: req,
  });
  return reply.data;
}

export function ping(): Promise<UploadResp> {
  const req: UploadReq = { kind: 'ping' };
  return call<UploadReq, UploadResp>('ipc_upload', req);
}

export async function listDestinations(): Promise<DestinationView[]> {
  const response = await destination({ kind: 'list' });
  return response.kind === 'destinations' ? response.data : [];
}

export async function addDestination(kind: DestinationKind): Promise<DestinationView> {
  const response = await destination({ kind: 'add', data: { kind } });
  if (response.kind !== 'destination') {
    throw new Error('The destination could not be added.');
  }
  return response.data;
}

export async function saveDestination(input: DestinationInput): Promise<DestinationView> {
  const response = await destination({ kind: 'save', data: { input } });
  if (response.kind !== 'destination') {
    throw new Error('The destination could not be saved.');
  }
  return response.data;
}

export async function removeDestination(id: string): Promise<void> {
  const response = await destination({ kind: 'remove', data: { id } });
  if (response.kind !== 'removed') {
    throw new Error('The destination could not be removed.');
  }
}

export async function testDestination(id: string): Promise<DestinationTestOutcome> {
  const response = await destination({ kind: 'test', data: { id } });
  if (response.kind !== 'test') {
    throw new Error('The test upload could not be started.');
  }
  return response.data.outcome;
}

export async function importSxcu(): Promise<ImportedDestination> {
  const response = await destination({ kind: 'importSxcu' });
  if (response.kind !== 'imported') {
    throw new Error('The uploader could not be imported.');
  }
  return response.data;
}

export async function exportSxcu(
  id: string,
  includeHeaderValues = false,
): Promise<string | null> {
  const response = await destination({
    kind: 'exportSxcu',
    data: { id, includeHeaderValues },
  });
  if (response.kind !== 'exported') {
    throw new Error('The uploader could not be exported.');
  }
  return response.data.path;
}
