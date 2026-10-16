/**
 * Detects Compose service names that would shadow one of litepod's own containers.
 *
 * Every deployed app joins the shared proxy network, and Compose registers each
 * service name as a DNS alias there. A service named `libsql` therefore answers to
 * the same name the control plane used to dial its own database, and a service
 * named `app` shadows the dashboard's upstream. The API refuses these names on
 * save, validate, and deploy; this mirrors the rule in the editor so the problem
 * is visible while typing.
 *
 * Deliberately a text scan rather than a YAML parse, matching the other compose
 * utilities: it has to keep working while the user is halfway through typing.
 */

export type ReservedService = {
	name: string;
	line: number;
	column: number;
	length: number;
};

/** Keep in sync with `RESERVED_SERVICE_NAMES` in `api/src/deployment/compose/mod.rs`. */
export const RESERVED_SERVICE_NAMES = [
	'api',
	'app',
	'buildkit',
	'caddy',
	'dragonfly',
	'libsql',
] as const;

const SERVICE_RE = /^(\s+)([\w.-]+)\s*:/;

export function findReservedServices(content: string): ReservedService[] {
	const lines = (content || '').split('\n');
	const found: ReservedService[] = [];
	const seen = new Set<string>();
	let inServices = false;

	for (let index = 0; index < lines.length; index++) {
		const line = lines[index];
		const trimmed = line.trim();

		if (trimmed.startsWith('#')) continue;
		if (trimmed.startsWith('services:')) {
			inServices = true;
			continue;
		}
		if (!inServices) continue;
		// A non-indented key ends the services block.
		if (trimmed !== '' && !line.startsWith(' ')) {
			inServices = false;
			continue;
		}

		const match = SERVICE_RE.exec(line);
		// Only the two-space service keys, not the deeper keys of a service body.
		if (!match || match[1].length !== 2) continue;

		const name = match[2].toLowerCase();
		if (!RESERVED_SERVICE_NAMES.includes(name as (typeof RESERVED_SERVICE_NAMES)[number])) continue;
		if (seen.has(name)) continue;
		seen.add(name);
		found.push({
			name: match[2],
			line: index + 1,
			column: match[1].length + 1,
			length: match[2].length,
		});
	}

	return found;
}
