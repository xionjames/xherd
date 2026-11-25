// The Chat / Terminal choice of each job, shared by the job page's tab label
// and the terminal panel (#202 R7). A choice is kept in this browser (`saveJobView`).
import { jobView, saveJobView } from './chat';
import type { JobView } from './types';

const chosen = $state<Record<string, JobView>>({});

export function viewOf(jobId: string, hasChat: boolean, configView: unknown): JobView {
  return hasChat ? (chosen[jobId] ?? jobView(jobId, hasChat, configView)) : 'terminal';
}

export function chooseView(jobId: string, v: JobView): void {
  chosen[jobId] = v;
  saveJobView(jobId, v);
}
