// Runtime plugins an agent can be started with (switched on).
import type { PluginInfo } from './types';

export const runtimes = (plugins: PluginInfo[] | undefined): PluginInfo[] =>
  (plugins ?? []).filter((p) => p.kind === 'runtime' && p.enabled);

export const runtimeLabel = (p: PluginInfo): string => p.label ?? p.name;
