import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { isNamedIcon, ORGANIZATION_ICON_MAP } from './organization-icons';

describe('icon names', () => {
	it('match the list the API validates and picks at random from', () => {
		const source = readFileSync('../api/src/icons/validate.rs', 'utf8');
		const list = source.slice(source.indexOf('NAMES: &[&str]'), source.indexOf('];'));
		const names = [...list.matchAll(/"([^"]+)"/g)].map((match) => match[1]);
		expect(names.sort()).toEqual(Object.keys(ORGANIZATION_ICON_MAP).sort());
	});

	it('tells a name from anything else', () => {
		expect(isNamedIcon('rocket')).toBe(true);
		expect(isNamedIcon('constructor')).toBe(false);
		expect(isNamedIcon('https://example.com/a.png')).toBe(false);
		expect(isNamedIcon(null)).toBe(false);
	});
});
