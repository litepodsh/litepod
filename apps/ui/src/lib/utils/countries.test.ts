import { describe, expect, it } from 'vitest';
import { atlasAlpha2, countryName, flag } from './countries';

describe('countries', () => {
	it('maps atlas ids and unnamed shapes to alpha-2', () => {
		expect(atlasAlpha2('484')).toBe('MX');
		expect(atlasAlpha2('840')).toBe('US');
		expect(atlasAlpha2(undefined, 'Kosovo')).toBe('XK');
		expect(atlasAlpha2('999')).toBeNull();
	});

	it('names countries in the UI locale', () => {
		expect(countryName('MX', 'es')).toBe('México');
		expect(countryName('de', 'en')).toBe('Germany');
	});

	it('builds flag emoji', () => {
		expect(flag('mx')).toBe('🇲🇽');
		expect(flag(null)).toBe('');
		expect(flag('EUR')).toBe('');
	});
});
