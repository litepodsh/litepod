// Mirrors `api/src/templates/secrets.rs` so the .env editor can mint a real secret the
// moment a `{{...}}` marker is picked, instead of waiting for the server to substitute
// it on save. Keep the grammar, bounds and alphabets in sync with that file.

const MIN_LEN = 1;
const MAX_LEN = 128;
const ALNUM = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789';
const ALNUM_SYMBOLS = `${ALNUM}!%^*-_+.,~`;
// No `I O l 0 1` — a password read off a screen cannot be mistranscribed — and no `=`,
// which would hide where the password starts on a `KEY=VALUE` line.
const READABLE = 'ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789-_!';
const HEX = '0123456789abcdef';
const HEX_UPPER = '0123456789ABCDEF';
const BASE64 = `${ALNUM}+/`;
const BASE64URL = `${ALNUM}-_`;
const UUID_CHARS = '0123456789abcdef-';

type Marker =
	| { kind: 'password'; len: number; alphabet: string }
	| { kind: 'hex'; bytes: number; upper: boolean }
	| { kind: 'base64'; bytes: number; urlSafe: boolean }
	| { kind: 'uuid'; v7: boolean };

function sized(rest: string): number | null {
	if (!/^\d+$/.test(rest)) return null;
	const size = Number(rest);
	return size >= MIN_LEN && size <= MAX_LEN ? size : null;
}

/** Parses a `{{...}}` marker. Returns null for anything unrecognized or out of range. */
export function parseMarker(marker: string): Marker | null {
	const trimmed = marker.trim();
	if (!trimmed.startsWith('{{') || !trimmed.endsWith('}}')) return null;
	const inner = trimmed.slice(2, -2).trim();

	if (inner === 'uuid_v4') return { kind: 'uuid', v7: false };
	if (inner === 'uuid_v7') return { kind: 'uuid', v7: true };

	for (const [prefix, build] of [
		['openssl_rand_', (n: number): Marker => ({ kind: 'hex', bytes: n, upper: false })],
		['hex_upper_', (n: number): Marker => ({ kind: 'hex', bytes: n, upper: true })],
		// base64url_ must be tried before base64_, which is not a prefix of it.
		['base64url_', (n: number): Marker => ({ kind: 'base64', bytes: n, urlSafe: true })],
		['base64_', (n: number): Marker => ({ kind: 'base64', bytes: n, urlSafe: false })],
	] as const) {
		if (inner.startsWith(prefix)) {
			const size = sized(inner.slice(prefix.length));
			return size === null ? null : build(size);
		}
	}
	if (inner.startsWith('password_')) {
		const rest = inner.slice('password_'.length);
		const [digits, alphabet] = rest.endsWith('_symbols')
			? [rest.slice(0, -'_symbols'.length), ALNUM_SYMBOLS]
			: rest.endsWith('_readable')
				? [rest.slice(0, -'_readable'.length), READABLE]
				: [rest, ALNUM];
		const len = sized(digits);
		return len === null ? null : { kind: 'password', len, alphabet };
	}
	return null;
}

/** The character set a marker's output is drawn from — used by the scramble animation. */
export function markerAlphabet(marker: string): string {
	const parsed = parseMarker(marker);
	if (!parsed) return ALNUM;
	switch (parsed.kind) {
		case 'hex':
			return parsed.upper ? HEX_UPPER : HEX;
		case 'base64':
			return parsed.urlSafe ? BASE64URL : BASE64;
		case 'uuid':
			return UUID_CHARS;
		case 'password':
			return parsed.alphabet;
	}
}

function randomBytes(count: number): Uint8Array {
	return crypto.getRandomValues(new Uint8Array(count));
}

function pick(alphabet: string, count: number): string {
	return Array.from(randomBytes(count), (byte) => alphabet[byte % alphabet.length]).join('');
}

function encodeBase64(bytes: Uint8Array, urlSafe: boolean): string {
	// Rust uses the padded URL_SAFE engine, so padding is kept in both variants.
	const standard = btoa(String.fromCharCode(...bytes));
	return urlSafe ? standard.replaceAll('+', '-').replaceAll('/', '_') : standard;
}

/** Generates the real secret a marker stands for, or null if it is not a valid marker. */
export function generateSecret(marker: string): string | null {
	const parsed = parseMarker(marker);
	if (!parsed) return null;
	switch (parsed.kind) {
		case 'hex': {
			const hex = Array.from(randomBytes(parsed.bytes), (byte) =>
				byte.toString(16).padStart(2, '0'),
			).join('');
			return parsed.upper ? hex.toUpperCase() : hex;
		}
		case 'base64':
			return encodeBase64(randomBytes(parsed.bytes), parsed.urlSafe);
		case 'uuid':
			// v7 is time-ordered: a millisecond timestamp, then randomness.
			return parsed.v7 ? uuidV7() : crypto.randomUUID();
		case 'password':
			return pick(parsed.alphabet, parsed.len);
	}
}

function uuidV7(): string {
	const bytes = randomBytes(16);
	const millis = Date.now();
	for (let index = 0; index < 6; index += 1) {
		bytes[index] = Number((BigInt(millis) >> BigInt(8 * (5 - index))) & 0xffn);
	}
	bytes[6] = (bytes[6] & 0x0f) | 0x70; // version 7
	bytes[8] = (bytes[8] & 0x3f) | 0x80; // RFC 4122 variant
	const hex = Array.from(bytes, (byte) => byte.toString(16).padStart(2, '0')).join('');
	return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
}

/**
 * Index of the `{{` that a completion should replace from, given the line text up to the
 * cursor — or null when the cursor is not inside a marker in a value position. Markers
 * are values, never keys, so a comment line or a cursor before the `=` yields nothing.
 */
export function markerStart(lineBeforeCursor: string): number | null {
	if (lineBeforeCursor.trimStart().startsWith('#')) return null;
	const separator = lineBeforeCursor.indexOf('=');
	if (separator === -1) return null;
	const opening = lineBeforeCursor.lastIndexOf('{{');
	return opening > separator ? opening : null;
}

export const SECRET_MARKER_PRESETS: { marker: string; detail: string }[] = [
	// Ordered by how often the catalog templates actually use each marker.
	{ marker: '{{password_32}}', detail: '32 random alphanumeric characters' },
	{ marker: '{{openssl_rand_32}}', detail: '32 random bytes, hex — 64 characters' },
	{ marker: '{{password_12}}', detail: '12 random alphanumeric characters' },
	{
		marker: '{{password_16_readable}}',
		detail: '16 characters with no look-alikes — for a password someone retypes',
	},
	{ marker: '{{openssl_rand_16}}', detail: '16 random bytes, hex — 32 characters' },
	{ marker: '{{openssl_rand_64}}', detail: '64 random bytes, hex — 128 characters' },
	{ marker: '{{hex_upper_32}}', detail: '32 random bytes, upper-case hex — 64 characters' },
	{ marker: '{{base64url_32}}', detail: '32 random bytes, URL-safe Base64' },
	{ marker: '{{base64_32}}', detail: '32 random bytes, standard Base64' },
	{ marker: '{{base64_64}}', detail: '64 random bytes, standard Base64' },
	{ marker: '{{password_32_symbols}}', detail: '32 random characters, alphanumeric + symbols' },
	{ marker: '{{uuid_v4}}', detail: 'Random UUID' },
	{ marker: '{{uuid_v7}}', detail: 'Time-ordered UUID' },
];

/** The marker grammar, for the info popover next to the editor. `N` is 1–128. */
export const SECRET_MARKER_DOCS: { syntax: string; detail: string }[] = [
	{ syntax: '{{password_N}}', detail: 'N alphanumeric characters' },
	{ syntax: '{{password_N_symbols}}', detail: 'N characters, alphanumeric + symbols' },
	{ syntax: '{{password_N_readable}}', detail: 'N characters, no look-alikes (no I O l 0 1)' },
	{ syntax: '{{openssl_rand_N}}', detail: 'N random bytes as hex — same as openssl rand -hex N' },
	{ syntax: '{{hex_upper_N}}', detail: 'The same, upper-case' },
	{ syntax: '{{base64_N}}', detail: 'N random bytes, standard Base64' },
	{ syntax: '{{base64url_N}}', detail: 'N random bytes, URL-safe Base64' },
	{ syntax: '{{uuid_v4}}', detail: 'Random UUID' },
	{ syntax: '{{uuid_v7}}', detail: 'Time-ordered UUID' },
];
