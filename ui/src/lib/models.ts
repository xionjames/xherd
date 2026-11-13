// Models per runtime (issue #112): the suggestions a runtime plugin lists, or
// `null` when the runtime cannot choose a model. Names stay free text.
import type { ModelSuggestion, PluginInfo } from './types';

export const runtimeModels = (plugins: PluginInfo[] | undefined, runtime: string): ModelSuggestion[] | null =>
  plugins?.find((p) => p.name === runtime && p.kind === 'runtime')?.models ?? null;

/** Same rule as the daemon (`xherd_core::model::clean`): it goes on a command line. */
export const validModel = (m: string): boolean => {
  const t = m.trim();
  return t === '' || (t.length <= 100 && !t.startsWith('-') && /^[A-Za-z0-9._:/@\-[\]]+$/.test(t));
};

export const modelText = (s: ModelSuggestion): string =>
  [s.label, s.tier].filter(Boolean).join(' · ');
