import { describe, expect, it } from 'vitest';

import { parseCatalog, validateCatalogs } from './catalogs';

describe('parseCatalog', () => {
	it('prefixes nested TOML keys with their file namespace', () => {
		expect(
			parseCatalog(
				'[navigation]\ntools = "Tools"\n[user_menu]\nwelcome = "Welcome, {name}"',
				'menu',
			),
		).toEqual({
			'menu.navigation.tools': 'Tools',
			'menu.user_menu.welcome': 'Welcome, {name}',
		});
	});

	it('throws for malformed TOML', () => {
		expect(() => parseCatalog('[navigation]\ntools = ', 'menu')).toThrow();
	});
});

describe('validateCatalogs', () => {
	it('reports missing locale keys as warnings and placeholder mismatches as errors', () => {
		const issues = validateCatalogs({
			en: { 'menu.user_menu.welcome': 'Welcome, {name}', 'menu.navigation.tools': 'Tools' },
			es: { 'menu.user_menu.welcome': 'Hola, {nombre}' },
		});

		expect(issues).toContainEqual({
			level: 'warning',
			code: 'missing-key',
			locale: 'es',
			key: 'menu.navigation.tools',
		});
		expect(issues).toContainEqual({
			level: 'error',
			code: 'placeholder-mismatch',
			key: 'menu.user_menu.welcome',
		});
	});
});
