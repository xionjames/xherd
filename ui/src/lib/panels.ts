// Panel registry (design §11): a panel type maps to a component. The agent
// page renders the built-in view below; declarative pack views come later.
import type { Component } from 'svelte';
import type { Agent } from './types';
import Documents from '../panels/Documents.svelte';
import Tasks from '../panels/Tasks.svelte';
import Commits from '../panels/Commits.svelte';
import Changes from '../panels/Changes.svelte';
import PullRequest from '../panels/PullRequest.svelte';
import Activity from '../panels/Activity.svelte';
import TerminalPanel from '../panels/Terminal.svelte';
import Memory from '../panels/Memory.svelte';

export const PANELS: Record<string, Component<{ agent: Agent }>> = {
  documents: Documents,
  tasks: Tasks,
  commits: Commits,
  diff: Changes,
  pr: PullRequest,
  log: Activity,
  terminal: TerminalPanel,
  memory: Memory,
};

export interface TabDef {
  id: string; // URL `s=`
  panel: string; // registry type
  label: string;
  icon: string;
  /** Shown for flow `none` agents too. */
  noFlow: boolean;
}

/** The built-in agent view (labels as in the design). */
export const AGENT_VIEW: TabDef[] = [
  { id: 'terminal', panel: 'terminal', label: 'Terminal', icon: 'term', noFlow: true },
  { id: 'spec', panel: 'documents', label: 'Spec', icon: 'doc', noFlow: false },
  { id: 'tasks', panel: 'tasks', label: 'Tasks', icon: 'tasks', noFlow: false },
  { id: 'commits', panel: 'commits', label: 'Commits', icon: 'commit', noFlow: false },
  { id: 'changes', panel: 'diff', label: 'Changes', icon: 'diff', noFlow: true },
  { id: 'pr', panel: 'pr', label: 'Pull request', icon: 'pr', noFlow: true },
  { id: 'memory', panel: 'memory', label: 'Shared memory', icon: 'doc', noFlow: false },
  { id: 'activity', panel: 'log', label: 'Activity', icon: 'activity', noFlow: true },
];

/** Tabs whose panel exists, filtered for flow `none`. */
export function agentTabs(a: Agent): TabDef[] {
  return AGENT_VIEW.filter((t) => PANELS[t.panel] && (a.flow || t.noFlow));
}
