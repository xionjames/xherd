import { describe, expect, it } from 'vitest';
import { nextRepo } from './repoFromTicket';

const T = 'https://github.com/acme/app';

describe('nextRepo', () => {
  it('fills an empty field from the ticket', () => {
    expect(nextRepo({ typed: '', filled: null, ticketRepo: T })).toEqual({ repo: T, filled: T });
    expect(nextRepo({ typed: '  ', filled: null, ticketRepo: T })).toEqual({ repo: T, filled: T });
  });
  it('never replaces what the user typed', () => {
    expect(nextRepo({ typed: '/work/app', filled: null, ticketRepo: T })).toEqual({ repo: '/work/app', filled: null });
    expect(nextRepo({ typed: '/work/app', filled: T, ticketRepo: T })).toEqual({ repo: '/work/app', filled: null });
  });
  it('follows a changed ticket while the field is its own fill', () => {
    const other = 'https://github.com/acme/other';
    expect(nextRepo({ typed: T, filled: T, ticketRepo: other })).toEqual({ repo: other, filled: other });
  });
  it('clearing the ticket removes only its own fill', () => {
    expect(nextRepo({ typed: T, filled: T, ticketRepo: null })).toEqual({ repo: '', filled: null });
    expect(nextRepo({ typed: '/work/app', filled: null, ticketRepo: null })).toEqual({ repo: '/work/app', filled: null });
    expect(nextRepo({ typed: '', filled: null, ticketRepo: null })).toEqual({ repo: '', filled: null });
  });
});
