import { describe, expect, it } from 'vitest';

import { createTranslator, detectLocale, prefersEuropeanFlags, setLocale, t } from './index.svelte';

describe('locale selection', () => {
	it('uses an explicit stored language before browser preferences', () => {
		expect(detectLocale('en', ['es-MX', 'en-US'])).toBe('en');
	});

	it('selects Spanish for a Spanish browser and English otherwise', () => {
		expect(detectLocale(null, ['es-MX', 'en-US'])).toBe('es');
		expect(detectLocale(null, ['fr-CA'])).toBe('en');
	});

	it('uses European flags only for a European browser region', () => {
		expect(prefersEuropeanFlags(['es-ES', 'en-US'])).toBe(true);
		expect(prefersEuropeanFlags(['en-GB'])).toBe(true);
		expect(prefersEuropeanFlags(['es-MX', 'en-US'])).toBe(false);
	});
});

describe('t', () => {
	it('interpolates named values and preserves a missing variable', () => {
		setLocale('en');
		expect(t('menu.user_menu.welcome', { name: 'Ada' })).toBe('Welcome, Ada');
		expect(t('menu.user_menu.welcome')).toBe('Welcome, {name}');
	});

	it('falls back in both directions and returns an unknown key', () => {
		const source = {
			en: { 'menu.english_only': 'English only' },
			es: { 'menu.spanish_only': 'Solo español' },
		} as const;

		expect(createTranslator(source, () => 'en')('menu.spanish_only')).toBe('Solo español');
		expect(createTranslator(source, () => 'es')('menu.english_only')).toBe('English only');
		expect(createTranslator(source, () => 'es')('menu.unknown')).toBe('menu.unknown');
	});
});
