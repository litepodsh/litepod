/**
 * Detects a top-level `version:` key in a Compose file.
 *
 * The `version` field is obsolete in Compose V2+ and Podman emits a deprecation
 * warning when it is present. litepod blocks deployment until it is removed.
 *
 * Deliberately a text scan rather than a YAML parse, matching the other compose
 * utilities: it has to keep working while the user is halfway through typing.
 */

export type DeprecatedVersion = {
	found: boolean;
	line: number;
	column: number;
	length: number;
	version: string;
};

const VERSION_RE = /^version\s*:\s*["']?([0-9]+(?:\.[0-9]+)*)["']?\s*$/;

export function hasDeprecatedComposeVersion(content: string): DeprecatedVersion {
	const lines = (content || '').split('\n');

	for (let index = 0; index < lines.length; index++) {
		const line = lines[index];
		const trimmed = line.trimStart();

		if (trimmed.startsWith('#')) continue;

		const match = VERSION_RE.exec(line);
		if (match) {
			return {
				found: true,
				line: index + 1,
				column: match.index + 1,
				length: match[0].length,
				version: match[1],
			};
		}
	}

	return { found: false, line: 0, column: 0, length: 0, version: '' };
}
