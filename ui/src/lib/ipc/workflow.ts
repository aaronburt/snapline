import type { WorkflowReq } from './bindings/WorkflowReq';
import type { WorkflowResp } from './bindings/WorkflowResp';
import { call } from './core';

export function ping(): Promise<WorkflowResp> {
  const req: WorkflowReq = { kind: 'ping' };
  return call<WorkflowReq, WorkflowResp>('ipc_workflow', req);
}
