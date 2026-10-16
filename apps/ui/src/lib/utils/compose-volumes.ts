/**
 * Finds host bind mounts in a Compose file and flags them.
 *
 * litepod stores Compose projects inside the `litepod_data` named volume, so paths
 * like `../files/db` only exist inside the API container. Podman resolves a
 * bind source on the host, where that path is absent, and the deploy dies with
 * `statfs <path>: no such file or directory`. Named volumes have no such
 * problem, so every host bind gets a warning pointing at the replacement.
 *
 * Deliberately a text scan rather than a YAML parse, matching
 * `compose-variables.ts`: it has to keep working while the user is halfway
 * through typing and the document does not parse.
 */

export type ComposeBindMount = {
	/** The host path as written, e.g. `../files/db`. */
	source: string;
	/** The container path, when the entry declared one. */
	target: string;
	/** 1-based, for Monaco markers. */
	line: number;
	/** 1-based, for Monaco markers. */
	column: number;
	/** Length of the source token, so the squiggle covers just the path. */
	length: number;
	/** Named volume we suggest instead, e.g. `db_data`. */
	suggestion: string;
};

export type ComposeUndeclaredVolume = {
	name: string;
	line: number;
	column: number;
	length: number;
};

/**
 * Compose only accepts this shape as a volume name. Everything else — `/abs`,
 * `./rel`, `.hidden`, `nested/dir`, `~/home`, `${VAR}` — is read as a host
 * path, so the check is "not a valid name" rather than a list of path prefixes.
 */
const NAMED_VOLUME = /^[A-Za-z0-9][A-Za-z0-9_.-]*$/;

const isHostPath = (source: string) => !NAMED_VOLUME.test(source);

/**
 * Tells long syntax (`type: bind`) from short syntax (`db_data:/var/lib/mysql`).
 * Both contain a colon; only a YAML mapping requires whitespace after it.
 */
const LONG_SYNTAX = /^[A-Za-z_][A-Za-z0-9_]*\s*:(\s|$)/;

const BLANK_OR_COMMENT = (line: string) => !line.trim() || line.trimStart().startsWith('#');

const indentOf = (line: string) => line.length - line.trimStart().length;

function unquote(value: string): string {
	const trimmed = value.trim();
	if (
		(trimmed.startsWith('"') && trimmed.endsWith('"') && trimmed.length > 1) ||
		(trimmed.startsWith("'") && trimmed.endsWith("'") && trimmed.length > 1)
	) {
		return trimmed.slice(1, -1);
	}
	return trimmed;
}

/**
 * Turns a mount into a plausible volume name: `../files/db` -> `db_data`,
 * `/var/lib/mysql` -> `mysql_data`. Compose namespaces it per project, so a
 * generic name is safe.
 */
function suggestName(source: string, target: string): string {
	const basename = (path: string) =>
		path
			.split('/')
			.filter((part) => part && part !== '.' && part !== '..')
			.pop() ?? '';
	const stem = basename(source) || basename(target) || 'app';
	const slug = stem
		.toLowerCase()
		.replace(/^\.+/, '') // `.db_data2` is a hidden path, not a name prefix
		.replace(/[^a-z0-9_]/g, '_')
		.replace(/^_+|_+$/g, '');
	if (!slug) return 'app_data';
	// `.db_data2` already reads as storage; `db` does not.
	return slug.includes('data') ? slug : `${slug}_data`;
}

/**
 * Splits `SOURCE:TARGET[:opts]` without tripping over `${VAR:-default}`, which
 * legitimately contains a colon inside its braces.
 */
function splitShortSyntax(entry: string): { source: string; target: string } | null {
	let depth = 0;
	const parts: string[] = [];
	let current = '';
	for (let index = 0; index < entry.length; index++) {
		const char = entry[index];
		if (char === '{') depth++;
		else if (char === '}') depth = Math.max(0, depth - 1);
		if (char === ':' && depth === 0) {
			parts.push(current);
			current = '';
			continue;
		}
		current += char;
	}
	parts.push(current);
	if (parts.length < 2) return null;
	return { source: parts[0], target: parts[1] };
}

type VolumeItem = {
	/** Line indices (0-based) making up this list entry. */
	lines: number[];
};

/**
 * Walks `services.<name>.volumes` blocks and returns their list entries.
 * The top-level `volumes:` declaration sits at indent 0 and is skipped.
 */
function serviceVolumeItems(lines: string[]): VolumeItem[] {
	const items: VolumeItem[] = [];
	let inServices = false;
	let volumesIndent: number | null = null;
	let current: VolumeItem | null = null;

	const closeItem = () => {
		if (current) items.push(current);
		current = null;
	};

	for (let index = 0; index < lines.length; index++) {
		const raw = lines[index];
		if (BLANK_OR_COMMENT(raw)) continue;
		const indent = indentOf(raw);
		const trimmed = raw.trim();

		if (indent === 0) {
			closeItem();
			volumesIndent = null;
			inServices = /^services:\s*$/.test(trimmed);
			continue;
		}
		if (!inServices) continue;

		// Leaving the volumes block: any key at or above its indent ends it.
		if (volumesIndent !== null && indent <= volumesIndent) {
			closeItem();
			volumesIndent = null;
		}

		if (/^volumes:\s*$/.test(trimmed)) {
			volumesIndent = indent;
			continue;
		}
		if (volumesIndent === null) continue;

		if (trimmed.startsWith('-')) {
			closeItem();
			current = { lines: [index] };
		} else if (current) {
			// Continuation of a long-syntax mapping.
			current.lines.push(index);
		}
	}
	closeItem();
	return items;
}

export function extractHostBindMounts(content: string): ComposeBindMount[] {
	const lines = (content || '').split('\n');
	const mounts: ComposeBindMount[] = [];

	for (const item of serviceVolumeItems(lines)) {
		const head = lines[item.lines[0]];
		const entry = head.trim().replace(/^-\s*/, '');

		// Long syntax: `- type: bind` with `source:` / `target:` keys.
		if (LONG_SYNTAX.test(entry)) {
			const fields = new Map<string, { value: string; line: number; column: number }>();
			for (const lineIndex of item.lines) {
				const text = lines[lineIndex];
				const match = text.match(/^\s*(?:-\s*)?([A-Za-z_][A-Za-z0-9_]*)\s*:\s*(.+?)\s*$/);
				if (!match) continue;
				const value = unquote(match[2]);
				fields.set(match[1], {
					value,
					line: lineIndex + 1,
					column: text.indexOf(match[2]) + 1,
				});
			}
			const type = fields.get('type')?.value;
			const source = fields.get('source');
			if (type !== 'bind' || !source) continue;
			const target = fields.get('target')?.value ?? '';
			mounts.push({
				source: source.value,
				target,
				line: source.line,
				column: source.column,
				length: source.value.length,
				suggestion: suggestName(source.value, target),
			});
			continue;
		}

		// Short syntax: `- SOURCE:TARGET[:opts]`.
		const parsed = splitShortSyntax(unquote(entry));
		if (!parsed) continue;
		const source = parsed.source.trim();
		if (!isHostPath(source)) continue;
		const column = head.indexOf(source) + 1;
		mounts.push({
			source,
			target: parsed.target.trim(),
			line: item.lines[0] + 1,
			column: column > 0 ? column : 1,
			length: source.length,
			suggestion: suggestName(source, parsed.target),
		});
	}

	return mounts;
}

/** Whether the file already has a top-level `volumes:` key to append to. */
export function hasRootVolumesBlock(content: string): boolean {
	return (content || '')
		.split('\n')
		.some((line) => indentOf(line) === 0 && /^volumes:\s*$/.test(line.trim()));
}

/**
 * Named volumes must also appear under the top-level `volumes:` key. Reporting
 * this alongside the bind warnings keeps the suggested fix from failing on the
 * next deploy.
 */
export function extractUndeclaredVolumes(content: string): ComposeUndeclaredVolume[] {
	const lines = (content || '').split('\n');
	const declared = new Set<string>();

	let inRootVolumes = false;
	// The name level is whatever indent the block actually uses, so 2- and
	// 4-space files both work.
	let nameIndent: number | null = null;
	for (const raw of lines) {
		if (BLANK_OR_COMMENT(raw)) continue;
		const indent = indentOf(raw);
		const trimmed = raw.trim();
		if (indent === 0) {
			inRootVolumes = /^volumes:\s*$/.test(trimmed);
			nameIndent = null;
			continue;
		}
		if (!inRootVolumes) continue;
		if (nameIndent === null) nameIndent = indent;
		// Only the first nesting level names a volume; deeper lines configure it.
		const match = trimmed.match(/^([A-Za-z0-9][A-Za-z0-9_.-]*)\s*:/);
		if (match && indent === nameIndent) declared.add(match[1]);
	}

	const undeclared = new Map<string, ComposeUndeclaredVolume>();
	for (const item of serviceVolumeItems(lines)) {
		const head = lines[item.lines[0]];
		const entry = unquote(head.trim().replace(/^-\s*/, ''));
		if (LONG_SYNTAX.test(entry)) continue;
		const parsed = splitShortSyntax(entry);
		if (!parsed) continue;
		const name = parsed.source.trim();
		if (!name || isHostPath(name) || declared.has(name)) continue;
		if (undeclared.has(name)) continue;
		const column = head.indexOf(name) + 1;
		undeclared.set(name, {
			name,
			line: item.lines[0] + 1,
			column: column > 0 ? column : 1,
			length: name.length,
		});
	}
	return [...undeclared.values()];
}
