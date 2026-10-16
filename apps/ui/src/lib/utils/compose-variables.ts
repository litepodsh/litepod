/**
 * Finds `${VAR}` references in a Compose file and diffs them against the
 * application's runtime variables.
 *
 * Deliberately a text scan rather than a YAML parse: it has to keep working
 * while the user is halfway through typing and the document does not parse.
 */

export type ComposeVarRef = {
	name: string;
	/** 1-based, for Monaco markers. */
	line: number;
	/** 1-based, for Monaco markers. */
	column: number;
	/** Length of the whole reference, so the squiggle covers `${VAR}`. */
	length: number;
	/** `:-` `-` `:+` `+` — Compose resolves these without a variable. */
	hasDefault: boolean;
	/** `:?` `?` — Compose fails when the variable is missing. */
	required: boolean;
	/** The API only accepts `[A-Z_][A-Z0-9_]*` variable names. */
	creatable: boolean;
};

// `$$` first so Compose's literal-dollar escape is consumed, not matched.
const REFERENCE =
	/\$\$|\$\{([A-Za-z_][A-Za-z0-9_]*)(?::?([-?+])[^}]*)?\}|\$([A-Za-z_][A-Za-z0-9_]*)/g;

const CREATABLE_NAME = /^[A-Z_][A-Z0-9_]*$/;

export function extractComposeVariableRefs(content: string): ComposeVarRef[] {
	const refs: ComposeVarRef[] = [];
	const lines = (content || '').split('\n');

	for (let index = 0; index < lines.length; index++) {
		const line = lines[index];
		if (line.trimStart().startsWith('#')) continue;

		REFERENCE.lastIndex = 0;
		let match: RegExpExecArray | null;
		while ((match = REFERENCE.exec(line)) !== null) {
			const name = match[1] ?? match[3];
			if (!name) continue; // `$$` escape
			const operator = match[2];
			refs.push({
				name,
				line: index + 1,
				column: match.index + 1,
				length: match[0].length,
				hasDefault: operator === '-' || operator === '+',
				required: operator === '?',
				creatable: CREATABLE_NAME.test(name),
			});
		}
	}

	return refs;
}

/**
 * Collapses references to one entry per name (keeping the first occurrence, so
 * the marker lands on the earliest mention) and drops the ones Compose can
 * resolve on its own.
 */
export function diffMissingVariables(refs: ComposeVarRef[], defined: Set<string>): ComposeVarRef[] {
	const missing = new Map<string, ComposeVarRef>();
	for (const ref of refs) {
		if (defined.has(ref.name)) continue;
		if (ref.hasDefault && !ref.required) continue;
		if (!missing.has(ref.name)) missing.set(ref.name, ref);
	}
	return [...missing.values()];
}
