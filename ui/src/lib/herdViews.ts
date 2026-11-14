// Herd view registry (issue #108): a view id maps to a component that owns
// the Herd page's body below its head. The first entry is the default (no
// `?layout=` in the URL). Views only read: they get every agent, unfiltered,
// the packs' flows, and `open` to go to an agent. Enabled view plugins
// (R9, R12) come after the built-ins as `plugin:<name>`, drawn in a
// sandboxed frame by `HerdPluginView`.
import type { Component } from 'svelte';
import type { Agent, FlowInfo, PluginInfo } from './types';
import HerdList from '../components/HerdList.svelte';
import StageColumns from '../components/StageColumns.svelte';
import HerdPluginView from '../components/HerdPluginView.svelte';

export interface HerdViewProps {
  agents: Agent[];
  flows: FlowInfo[];
  open: (a: Agent) => void;
  /** For a plugin view: the plugin, its page and its label. */
  plugin?: string;
  entry?: string;
  label?: string;
}

export interface HerdView {
  id: string; // URL `layout=`
  label: string;
  component: Component<HerdViewProps>;
  /** A view plugin's name and entry page. */
  plugin?: string;
  entry?: string;
}

/** The built-in views. */
export const HERD_VIEWS: HerdView[] = [
  { id: 'list', label: 'List', component: HerdList },
  { id: 'stages', label: 'By stage', component: StageColumns },
];

/** The built-in views, then one per enabled view plugin. */
export function herdViews(plugins: readonly PluginInfo[] | undefined): HerdView[] {
  const extra = (plugins ?? [])
    .filter((p) => p.kind === 'view' && p.enabled && p.view?.entry)
    .map((p) => ({
      id: `plugin:${p.name}`,
      label: p.label || p.name,
      component: HerdPluginView,
      plugin: p.name,
      entry: p.view!.entry,
    }));
  return [...HERD_VIEWS, ...extra];
}

/** The view for a `layout` value; the first one when it is unknown, missing or switched off. */
export function pickView(id: string | null | undefined, plugins?: readonly PluginInfo[]): HerdView {
  const views = herdViews(plugins);
  return views.find((v) => v.id === id) ?? views[0];
}
