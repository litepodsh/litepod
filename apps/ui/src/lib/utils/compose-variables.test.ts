import { describe, expect, it } from 'vitest';

import { diffMissingVariables, extractComposeVariableRefs } from './compose-variables';

const names = (content: string) => extractComposeVariableRefs(content).map((ref) => ref.name);

describe('extractComposeVariableRefs', () => {
	it('finds braced and bare references', () => {
		expect(names('      - MP_UI_AUTH=${MP_UI_AUTH}\n      - HOME=$HOME')).toEqual([
			'MP_UI_AUTH',
			'HOME',
		]);
	});

	it('reports 1-based positions covering the whole reference', () => {
		const [ref] = extractComposeVariableRefs('a: 1\nb: ${PORT}');
		expect(ref).toMatchObject({ name: 'PORT', line: 2, column: 4, length: 7 });
	});

	it('flags default operators without treating them as required', () => {
		for (const source of ['${PORT:-3000}', '${PORT-3000}', '${PORT:+on}', '${PORT+on}']) {
			const [ref] = extractComposeVariableRefs(source);
			expect(ref, source).toMatchObject({ hasDefault: true, required: false });
		}
	});

	it('flags the error operator as required', () => {
		for (const source of ['${TOKEN:?missing}', '${TOKEN?missing}']) {
			const [ref] = extractComposeVariableRefs(source);
			expect(ref, source).toMatchObject({ hasDefault: false, required: true });
		}
	});

	it('ignores the $$ literal-dollar escape', () => {
		expect(names('- LITERAL=$${NOPE}')).toEqual([]);
		expect(names('- LITERAL=$$NOPE')).toEqual([]);
	});

	it('ignores comment lines', () => {
		expect(names('  # - TOKEN=${TOKEN}\n  - REAL=${REAL}')).toEqual(['REAL']);
	});

	it('marks non-uppercase names as not creatable', () => {
		const [lower, upper] = extractComposeVariableRefs('${my_var} ${MY_VAR}');
		expect(lower).toMatchObject({ name: 'my_var', creatable: false });
		expect(upper).toMatchObject({ name: 'MY_VAR', creatable: true });
	});

	it('keeps every occurrence', () => {
		expect(names('${A}\n${A}')).toEqual(['A', 'A']);
	});
});

describe('diffMissingVariables', () => {
	const refs = extractComposeVariableRefs(
		[
			'      - MP_UI_AUTH=${MP_UI_AUTH}',
			'      - PORT=${PORT:-3000}',
			'      - TOKEN=${TOKEN:?required}',
			'      - LOWER=${my_var}',
			'      - LITERAL=$${NOPE}',
			'      - KNOWN=${DATABASE_URL}',
			'      - AGAIN=${MP_UI_AUTH}',
		].join('\n'),
	);

	it('drops defined names, keeps undefined and required ones', () => {
		const missing = diffMissingVariables(refs, new Set(['DATABASE_URL']));
		expect(missing.map((ref) => ref.name)).toEqual(['MP_UI_AUTH', 'TOKEN', 'my_var']);
	});

	it('collapses repeats onto the first occurrence', () => {
		const [first] = diffMissingVariables(refs, new Set());
		expect(first).toMatchObject({ name: 'MP_UI_AUTH', line: 1 });
	});

	it('keeps a defaulted variable out even when nothing is defined', () => {
		expect(diffMissingVariables(refs, new Set()).map((ref) => ref.name)).not.toContain('PORT');
	});
});
