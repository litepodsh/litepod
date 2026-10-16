/**
 * Suggests an application name for an internal template when the dialog opens:
 * `{kind}-{4 random alphanumerics}`, e.g. `postgres-a3f9` or `mongodb-x7k2`.
 * The user can accept it as-is or overwrite it — the dialog never leaves the
 * name empty.
 */
export function suggestedResourceName(kind: string): string {
	const bytes = crypto.getRandomValues(new Uint8Array(4));
	const alphabet = 'abcdefghijklmnopqrstuvwxyz0123456789';
	const suffix = Array.from(bytes, (byte) => alphabet[byte % alphabet.length]).join('');
	return `${kind}-${suffix}`;
}
