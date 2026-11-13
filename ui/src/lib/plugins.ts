// Pure helpers of the plugin view (Welcome step 2 and Settings → Plugins).
import type { Check, PluginInfo } from './types';

/** Group headings, by plugin kind. The kind `host` is a "Git provider" in the UI. */
export const KIND_LABEL: Record<string, string> = {
  runtime: 'Coding agents',
  host: 'Git providers',
  tracker: 'Issue trackers',
  toolchain: 'Languages',
};

/** One plugin of the kind. */
export const KIND_ONE: Record<string, string> = {
  runtime: 'Coding agent',
  host: 'Git provider',
  tracker: 'Issue tracker',
  toolchain: 'Language',
};

export const KIND_ORDER = ['runtime', 'host', 'tracker', 'toolchain'];

/** Kinds that may have every plugin switched off. */
export const OPTIONAL_KINDS = ['tracker', 'host'];

/** The last plugin on in a kind that must keep one cannot be switched off. */
export const lastOn = (p: PluginInfo, plugins: PluginInfo[]) =>
  !OPTIONAL_KINDS.includes(p.kind) && p.enabled && plugins.filter((q) => q.kind === p.kind && q.enabled).length === 1;

/** Search: case-insensitive substring of the name, label, description, kind or its labels. */
export function matches(p: PluginInfo, query: string): boolean {
  const q = query.trim().toLowerCase();
  if (!q) return true;
  return [p.name, p.label, p.description, p.kind, KIND_LABEL[p.kind], KIND_ONE[p.kind]].some((s) => s?.toLowerCase().includes(q));
}

/** Enabled plugins with a login whose `plugin:<name>:login` check ran and failed. */
export function needLogin(plugins: PluginInfo[], checks: Check[] | null): PluginInfo[] {
  return plugins.filter((p) => {
    if (!p.enabled || !p.login) return false;
    const c = checks?.find((c) => c.id === `plugin:${p.name}:login`);
    return !!c && !c.ok;
  });
}
