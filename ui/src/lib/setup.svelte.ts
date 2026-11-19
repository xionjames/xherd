// Setup checks and the saved config (welcome flag). Until every required
// check passes, the UI stays on the Setup screen (spec §17.2).
import { get, post } from './api';
import type { Check, ConfigView, PackChoice, PackFile } from './types';

export const setup = $state<{ checks: Check[] | null; config: ConfigView | null; running: boolean }>({
  checks: null,
  config: null,
  running: false,
});

export const failingRequired = () => (setup.checks ?? []).filter((c) => c.required && !c.ok);

export async function loadChecks() {
  setup.running = true;
  try {
    setup.checks = (await get<{ checks: Check[] }>('/checks')).checks;
  } finally {
    setup.running = false;
  }
}

export async function fixCheck(id: string) {
  setup.running = true;
  try {
    setup.checks = (await post<{ checks: Check[] }>(`/checks/${encodeURIComponent(id)}/fix`)).checks;
  } finally {
    setup.running = false;
  }
}

/** The edited and obsolete pack files that wait for a decision (R5). */
export async function loadPackReview() {
  return (await get<{ files: PackFile[] }>('/install/review')).files;
}

/** Send the choices; the answer carries the re-run checks (R6). */
export async function resolvePackFiles(choices: PackChoice[]) {
  setup.running = true;
  try {
    setup.checks = (await post<{ checks: Check[] }>('/install/resolve', { choices })).checks;
  } finally {
    setup.running = false;
  }
}

export async function loadConfig() {
  setup.config = await get<ConfigView>('/config');
}
