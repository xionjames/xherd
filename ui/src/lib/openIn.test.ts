import { describe, expect, it } from 'vitest';
import { badLinkUrl, handlerState, installText, linkId } from './openIn';
import type { HandlerRecord, OpenInView } from './types';

const rec: HandlerRecord = { version: 1, xherd: '0.3.0', os: 'linux', terminals: ['konsole'], root: '/home/u/xherd/worktrees', at: 1 };
const view = (installed: HandlerRecord | null, updateAvailable = false, rootChanged = false): Pick<OpenInView, 'handler'> => ({
  handler: { shipped: 1, installed, updateAvailable, rootChanged },
});

describe('handlerState', () => {
  it('tells missing, installed and update apart', () => {
    expect(handlerState(view(null))).toBe('missing');
    expect(handlerState(view(rec))).toBe('installed');
    expect(handlerState(view(rec, true))).toBe('update');
    expect(handlerState(view(rec, false, true))).toBe('update');
  });
});

describe('installText', () => {
  it('is a button natively', () => {
    expect(installText('http://127.0.0.1:4740', true)).toEqual({ native: true });
  });
  it('is the curl commands from the origin in Docker', () => {
    expect(installText('http://127.0.0.1:4740/', false)).toEqual({
      native: false,
      install: 'curl -fsSL http://127.0.0.1:4740/api/v1/open-in/terminal-handler.sh | sh',
      uninstall: 'curl -fsSL http://127.0.0.1:4740/api/v1/open-in/terminal-handler.sh | sh -s uninstall',
    });
  });
});

describe('badLinkUrl', () => {
  it('accepts local-app schemes', () => {
    for (const u of ['idea://open?file={dir}', 'jetbrains://idea/navigate?path={dir}', 'x-github-client://openRepo/{dir}', 'Fork:open'])
      expect(badLinkUrl(u)).toBeNull();
  });
  it('refuses web, browser and xherd schemes in any case', () => {
    for (const s of ['http', 'HTTPS', 'ws', 'wss', 'ftp', 'JavaScript', 'data', 'vbscript', 'blob', 'file', 'about', 'xherd-term'])
      expect(badLinkUrl(`${s}:x`)).toMatch(/not allowed/);
  });
  it('needs a scheme and no spaces', () => {
    expect(badLinkUrl('/usr/bin/idea')).toMatch(/scheme/);
    expect(badLinkUrl('1x://a')).toMatch(/scheme/);
    expect(badLinkUrl('idea://a b')).toMatch(/spaces/);
    expect(badLinkUrl('idea://a\u0007')).toMatch(/control/);
    expect(badLinkUrl(`idea://${'a'.repeat(2048)}`)).toMatch(/2048/);
  });
});

describe('linkId', () => {
  it('follows the name and stays unique', () => {
    expect(linkId('IntelliJ IDEA', [])).toBe('intellij-idea');
    expect(linkId('IntelliJ IDEA', ['intellij-idea'])).toBe('intellij-idea-2');
    expect(linkId('!!!', [])).toBe('link');
  });
});
