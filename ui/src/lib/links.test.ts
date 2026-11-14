import { describe, expect, it } from 'vitest';
import { linkFor, segments, setLinkRules, ticketPill } from './links';

const jira = { plugin: 'jira', pattern: '\\b[A-Z][A-Z0-9]{1,9}-\\d+\\b', url: 'https://acme.atlassian.net/browse/{0}' };
const gh = { plugin: 'github-issues', pattern: '\\b([A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+)#(\\d+)\\b', url: 'https://github.com/{1}/issues/{2}' };

describe('link rules', () => {
  it('links keys and references inside text', () => {
    setLinkRules([jira, gh]);
    expect(segments('Fixes PAY-412 and acme/app#9.')).toEqual([
      { text: 'Fixes ' },
      { text: 'PAY-412', href: 'https://acme.atlassian.net/browse/PAY-412' },
      { text: ' and ' },
      { text: 'acme/app#9', href: 'https://github.com/acme%2Fapp/issues/9' },
      { text: '.' },
    ]);
  });

  it('links a whole ticket key and nothing else', () => {
    setLinkRules([jira]);
    expect(linkFor('PAY-412')).toBe('https://acme.atlassian.net/browse/PAY-412');
    expect(linkFor('pay-412')).toBeNull();
    expect(linkFor('see PAY-412')).toBeNull();
  });

  it('never makes a link that is not https, and survives a bad pattern or no rules', () => {
    setLinkRules([{ plugin: 'x', pattern: 'ab', url: 'javascript:alert({0})' }, { plugin: 'y', pattern: '(', url: 'https://x/{0}' }]);
    expect(segments('ab')).toEqual([{ text: 'ab' }]);
    setLinkRules([]);
    expect(segments('PAY-1')).toEqual([{ text: 'PAY-1' }]);
  });

  it('percent-encodes what it puts in the URL', () => {
    setLinkRules([{ plugin: 'x', pattern: '#(\\S+)', url: 'https://x/{1}' }]);
    expect(segments('#a/b?c')[0].href).toBe('https://x/a%2Fb%3Fc');
  });

  it('shows a ticket as its short id, linked to its URL or its key', () => {
    setLinkRules([jira]);
    expect(ticketPill(undefined)).toBeNull();
    expect(ticketPill({ key: 'PAY-412' })).toEqual({ label: 'PAY-412', href: 'https://acme.atlassian.net/browse/PAY-412' });
    expect(ticketPill({ key: '#12', url: 'https://github.com/a/b/issues/12' })).toEqual({ label: '#12', href: 'https://github.com/a/b/issues/12' });
    expect(ticketPill({ url: 'https://example.org/t/1' })).toEqual({ label: 'example.org', href: 'https://example.org/t/1' });
    expect(ticketPill({ key: 'X-1', url: 'http://example.org/t/1' })).toEqual({ label: 'X-1', href: null });
  });
});
