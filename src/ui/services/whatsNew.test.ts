import { describe, expect, it } from 'vitest';
import { compareVersions, parseChangelog, sectionsToShow } from './whatsNew';

const MD = `# Changelog

intro

## [Unreleased]

- next

## [0.3.0]

- **New**: a
  continued

## [0.2.0]

- b

## [0.1.0]

- c
`;

describe('whatsNew', () => {
  const s = parseChangelog(MD);
  it('parses released sections, newest first', () => {
    expect(s.map((x) => x.version)).toEqual(['0.3.0', '0.2.0', '0.1.0']);
    expect(s[0].body).toBe('- **New**: a\n  continued');
  });
  it('compares versions numerically', () => {
    expect(compareVersions('0.10.0', '0.9.1')).toBeGreaterThan(0);
    expect(compareVersions('1.0', '1.0.0')).toBe(0);
  });
  it('shows everything since the last run', () => {
    expect(sectionsToShow(s, '0.3.0', '0.1.0', true).map((x) => x.version)).toEqual(['0.3.0', '0.2.0']);
    expect(sectionsToShow(s, '0.3.0', '0.3.0', true)).toEqual([]);
    expect(sectionsToShow(s, '0.2.0', '0.3.0', true)).toEqual([]);
  });
  it('shows the current notes after an update from a version that did not record it, nothing on a fresh install', () => {
    expect(sectionsToShow(s, '0.3.0', null, true).map((x) => x.version)).toEqual(['0.3.0']);
    expect(sectionsToShow(s, '0.3.0', null, false)).toEqual([]);
  });
});
