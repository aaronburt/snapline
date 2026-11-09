import type { DestinationOption } from './bindings/DestinationOption';
import type { StepDto } from './bindings/StepDto';
import type { ViolationDto } from './bindings/ViolationDto';
import type { WorkflowInfo } from './bindings/WorkflowInfo';
import type { WorkflowInput } from './bindings/WorkflowInput';
import type { WorkflowReq } from './bindings/WorkflowReq';
import type { WorkflowResp } from './bindings/WorkflowResp';
import type { IpcError } from './core';
import { call } from './core';

export interface WorkflowList {
  workflows: WorkflowInfo[];
  destinations: DestinationOption[];
}

function unexpected(): IpcError {
  return {
    code: 'internal',
    message: 'Snapline returned an unexpected workflow answer.',
  };
}

function workflowOf(resp: WorkflowResp): WorkflowInfo {
  if (resp.kind === 'workflow') {
    return resp.data.workflow;
  }
  throw unexpected();
}

export function ping(): Promise<WorkflowResp> {
  const req: WorkflowReq = { kind: 'ping' };
  return call<WorkflowReq, WorkflowResp>('ipc_workflow', req);
}

export async function list(): Promise<WorkflowList> {
  const req: WorkflowReq = { kind: 'list' };
  const resp = await call<WorkflowReq, WorkflowResp>('ipc_workflow', req);
  if (resp.kind === 'list') {
    return resp.data;
  }
  throw unexpected();
}

export async function get(id: string): Promise<WorkflowInfo> {
  const req: WorkflowReq = { kind: 'get', data: { id } };
  return workflowOf(await call<WorkflowReq, WorkflowResp>('ipc_workflow', req));
}

export async function create(workflow: WorkflowInput): Promise<WorkflowInfo> {
  const req: WorkflowReq = { kind: 'create', data: { workflow } };
  return workflowOf(await call<WorkflowReq, WorkflowResp>('ipc_workflow', req));
}

export async function update(id: string, workflow: WorkflowInput): Promise<WorkflowInfo> {
  const req: WorkflowReq = { kind: 'update', data: { id, workflow } };
  return workflowOf(await call<WorkflowReq, WorkflowResp>('ipc_workflow', req));
}

export async function remove(id: string): Promise<string> {
  const req: WorkflowReq = { kind: 'delete', data: { id } };
  const resp = await call<WorkflowReq, WorkflowResp>('ipc_workflow', req);
  if (resp.kind === 'deleted') {
    return resp.data.id;
  }
  throw unexpected();
}

export async function duplicate(id: string): Promise<WorkflowInfo> {
  const req: WorkflowReq = { kind: 'duplicate', data: { id } };
  return workflowOf(await call<WorkflowReq, WorkflowResp>('ipc_workflow', req));
}

export async function validate(
  id: string | null,
  workflow: WorkflowInput,
): Promise<ViolationDto[]> {
  const req: WorkflowReq = { kind: 'validate', data: { id, workflow } };
  const resp = await call<WorkflowReq, WorkflowResp>('ipc_workflow', req);
  if (resp.kind === 'validation') {
    return resp.data.violations;
  }
  throw unexpected();
}

export type { DestinationOption, StepDto, ViolationDto, WorkflowInfo, WorkflowInput };
