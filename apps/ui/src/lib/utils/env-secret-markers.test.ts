import { describe, expect, it } from 'vitest';

import { generateSecret, markerAlphabet, markerStart, parseMarker } from './env-secret-markers';

describe('generateSecret', () => {
	it('generates an alphanumeric password of the requested length', () => {
		const value = generateSecret('{{password_24}}')!;
		expect(value).toHaveLength(24);
		expect(value).toMatch(/^[A-Za-z0-9]+$/);
	});

	it('allows symbols only when the marker asks for them', () => {
		expect(generateSecret('{{password_40_symbols}}')).toHaveLength(40);
		expect(generateSecret('{{password_40_symbols}}')).toMatch(/^[A-Za-z0-9!%^*\-_+.,~]+$/);
	});

	it('generates hex matching the openssl rand convention', () => {
		const value = generateSecret('{{openssl_rand_32}}')!;
		expect(value).toHaveLength(64);
		expect(value).toMatch(/^[0-9a-f]+$/);
	});

	it('generates padded standard base64 from the requested number of bytes', () => {
		const value = generateSecret('{{base64_32}}')!;
		expect(atob(value)).toHaveLength(32);
		expect(value.endsWith('=')).toBe(true);
	});

	it('generates padded url-safe base64 from the requested number of bytes', () => {
		const value = generateSecret('{{base64url_32}}')!;
		expect(value).not.toMatch(/[+/]/);
		expect(value.endsWith('=')).toBe(true);
		expect(atob(value.replaceAll('-', '+').replaceAll('_', '/'))).toHaveLength(32);
	});

	it('tolerates whitespace inside and around the braces', () => {
		expect(generateSecret('  {{ password_8 }}  ')).toHaveLength(8);
	});

	it('returns null for malformed, unknown or out-of-range markers', () => {
		for (const input of [
			'{{password_}}',
			'{{password_0}}',
			'{{password_129}}',
			'{{unknown_9}}',
			'{{password_16}',
			'{{openssl_32}}',
			'{{base64_32_symbols}}',
			'changeme',
			'',
		]) {
			expect(generateSecret(input), input).toBeNull();
		}
	});
});

describe('markerAlphabet', () => {
	it('matches the characters the marker actually produces', () => {
		expect(markerAlphabet('{{openssl_rand_8}}')).toBe('0123456789abcdef');
		expect(markerAlphabet('{{base64url_8}}')).toContain('-_');
		expect(markerAlphabet('{{password_8_symbols}}')).toContain('!%^');
		expect(markerAlphabet('{{password_8}}')).not.toContain('!');
	});
});

describe('parseMarker', () => {
	it('prefers base64url over base64 on the shared prefix', () => {
		expect(parseMarker('{{base64url_8}}')).toEqual({ kind: 'base64', bytes: 8, urlSafe: true });
		expect(parseMarker('{{base64_8}}')).toEqual({ kind: 'base64', bytes: 8, urlSafe: false });
	});
});

describe('new markers', () => {
	it('generates upper-case hex', () => {
		const value = generateSecret('{{hex_upper_32}}')!;
		expect(value).toHaveLength(64);
		expect(value).toMatch(/^[0-9A-F]+$/);
	});

	it('omits look-alike characters from a readable password', () => {
		for (let attempt = 0; attempt < 64; attempt += 1) {
			const value = generateSecret('{{password_32_readable}}')!;
			expect(value).toHaveLength(32);
			expect(value).not.toMatch(/[IOl01=]/);
		}
	});

	it('generates uuids of the requested version', () => {
		expect(generateSecret('{{uuid_v4}}')).toMatch(
			/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/,
		);
		expect(generateSecret('{{uuid_v7}}')).toMatch(
			/^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/,
		);
	});

	it('sorts v7 uuids by creation order', () => {
		const first = generateSecret('{{uuid_v7}}')!;
		const second = generateSecret('{{uuid_v7}}')!;
		expect(first.slice(0, 13) <= second.slice(0, 13)).toBe(true);
	});

	it('rejects a uuid marker with a length or an unknown version', () => {
		expect(generateSecret('{{uuid_v9}}')).toBeNull();
		expect(generateSecret('{{uuid_v4_32}}')).toBeNull();
	});
});

describe('markerStart', () => {
	it('locates the marker being typed in a value position', () => {
		expect(markerStart('API_KEY={{')).toBe(8);
		expect(markerStart('API_KEY={{openssl_rand_')).toBe(8);
		expect(markerStart('URL=postgres://u:{{pass')).toBe(17);
	});

	it('ignores comments, keys and text outside a marker', () => {
		expect(markerStart('# API_KEY={{')).toBeNull();
		expect(markerStart('  # {{')).toBeNull();
		expect(markerStart('{{API_KEY')).toBeNull();
		expect(markerStart('API_KEY=')).toBeNull();
		expect(markerStart('API_KEY=plain')).toBeNull();
	});
});
