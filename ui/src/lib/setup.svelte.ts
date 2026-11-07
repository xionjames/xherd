// Setup checks and the saved config (welcome flag). Until every required
// check passes, the UI stays on the Setup screen (spec §17.2).
import { get, post } from './api';
import type { Check, ConfigView } from './types';

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

export async function loadConfig() {
  setup.config = await get<ConfigView>('/config');
}
