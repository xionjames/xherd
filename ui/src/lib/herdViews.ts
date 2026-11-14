// Herd view registry (issue #108): a view id maps to a component that owns
// the Herd page's body below its head. The first entry is the default (no
// `?layout=` in the URL). Views only read: they get every agent, unfiltered,
// the packs' flows, and `open` to go to an agent.
import type { Component } from 'svelte';
import type { Agent, FlowInfo } from './types';
import HerdList from '../components/HerdList.svelte';
import StageColumns from '../components/StageColumns.svelte';
import HerdTiles from '../components/HerdTiles.svelte';

export interface HerdViewProps {
  agents: Agent[];
  flows: FlowInfo[];
  open: (a: Agent) => void;
}

export interface HerdView {
  id: string; // URL `layout=`
  label: string;
  component: Component<HerdViewProps>;
}

export const HERD_VIEWS: HerdView[] = [
  { id: 'list', label: 'List', component: HerdList },
  { id: 'stages', label: 'By stage', component: StageColumns },
  { id: 'tiles', label: 'Tiles', component: HerdTiles }, // a small example view
];

/** The view for a `layout` value; the first one when it is unknown or missing. */
export function pickView(id: string | null | undefined): HerdView {
  return HERD_VIEWS.find((v) => v.id === id) ?? HERD_VIEWS[0];
}
