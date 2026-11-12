// Which git provider a host is bound to (D24): the one named on its row, else
// the first whose host patterns claim its name (the daemon does the same).
import type { GitHost, GitProvider } from './types';

export const claims = (p: GitProvider, host: string) =>
  p.hosts.some((g) =>
    new RegExp('^' + g.split('*').map((x) => x.replace(/[.+?^${}()|[\]\\]/g, '\\$&')).join('.*') + '$', 'i').test(host));

export const boundProvider = (providers: GitProvider[], h: Pick<GitHost, 'host' | 'provider'>) =>
  h.provider ? providers.find((p) => p.name === h.provider) : providers.find((p) => claims(p, h.host));
