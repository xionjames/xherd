// Models per runtime (issue #112). Whether a runtime takes a model is in
// `/state`; its suggestions (a fixed list, or what its list command prints)
// come from `GET /runtimes/{name}/models`. Names stay free text.
import { get } from './api';
import type { ModelSuggestion, PluginInfo } from './types';

export const canChooseModel = (plugins: PluginInfo[] | undefined, runtime: string): boolean =>
  !!plugins?.find((p) => p.name === runtime && p.kind === 'runtime')?.chooseModel;

/** Same rule as the daemon (`xherd_core::model::clean`): it goes on a command line. */
export const validModel = (m: string): boolean => {
  const t = m.trim();
  return t === '' || (t.length <= 100 && !t.startsWith('-') && /^[A-Za-z0-9._:/@\-[\]]+$/.test(t));
};

/** What an option shows: the label (else the id) and the tier. */
export const modelText = (s: ModelSuggestion): string =>
  [s.label || s.id, s.tier].filter(Boolean).join(' · ');

export type ModelList = { choose: boolean; models: ModelSuggestion[]; error?: string };

// One request per runtime while the page is open (a list command may be slow);
// a failed one is asked again next time.
const lists = new Map<string, Promise<ModelList>>();
export function modelList(runtime: string): Promise<ModelList> {
  let p = lists.get(runtime);
  if (!p) {
    p = get<ModelList>(`/runtimes/${encodeURIComponent(runtime)}/models`);
    lists.set(runtime, p);
    p.then((l) => { if (l.error) lists.delete(runtime); }, () => lists.delete(runtime));
  }
  return p;
}
