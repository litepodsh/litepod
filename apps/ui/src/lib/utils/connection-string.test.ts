import { describe, expect, it } from 'vitest';
import { maskConnectionStringPassword } from './connection-string';

describe('maskConnectionStringPassword', () => {
	it('masks only the password in named-user connection strings', () => {
		expect(
			maskConnectionStringPassword('mariadb://mariadb:pa%3Ass%2F%40word@db.internal:3306/app'),
		).toBe('mariadb://mariadb:••••@db.internal:3306/app');
	});

	it('preserves the scheme for password-only Redis userinfo', () => {
		expect(maskConnectionStringPassword('redis://:secret@cache.internal:6379')).toBe(
			'redis://:••••@cache.internal:6379',
		);
	});

	it('leaves connection strings without a userinfo password unchanged', () => {
		expect(maskConnectionStringPassword('redis://cache.internal:6379')).toBe(
			'redis://cache.internal:6379',
		);
		expect(maskConnectionStringPassword('https://user@example.com/path:with-colon')).toBe(
			'https://user@example.com/path:with-colon',
		);
	});
});
