// @vitest-environment node
import { readFileSync, readdirSync } from 'node:fs';
import { join, relative } from 'node:path';
import { expect, it } from 'vitest';

// Guards against buttons that render but do nothing (GitHub #22): a button inside a <label>
// never opens the label's input, and our <Button> defaults to type="button", so inside a form
// it does not submit either.
const root = join(process.cwd(), 'src');
const ACTION =
	/onclick|onmouseenter|onfocus|onpointerdown|href=|type=["']?submit|\{\.\.\.|\bform=/;
// Buttons whose job is to take focus, e.g. to reveal a tooltip.
const ALLOWED = new Set(['lib/components/home/apps-table.svelte']);

function svelteFiles(dir: string): string[] {
	return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
		const path = join(dir, entry.name);
		if (entry.isDirectory()) return entry.name === 'ui' ? [] : svelteFiles(path);
		return entry.name.endsWith('.svelte') ? [path] : [];
	});
}

/** Opening tags of `name`, skipping `>` inside `{...}` and quotes. */
function openingTags(source: string, name: string): { index: number; tag: string }[] {
	const tags: { index: number; tag: string }[] = [];
	let from = 0;
	while ((from = source.indexOf(`<${name}`, from)) !== -1) {
		const start = from;
		let depth = 0;
		let quote = '';
		let end = start + 1;
		for (; end < source.length; end += 1) {
			const char = source[end];
			if (quote) quote = char === quote ? '' : quote;
			else if (depth === 0 && (char === '"' || char === "'")) quote = char;
			else if (char === '{') depth += 1;
			else if (char === '}') depth -= 1;
			else if (char === '>' && depth === 0) break;
		}
		if (/[\s/>]/.test(source[start + name.length + 1] ?? ''))
			tags.push({ index: start, tag: source.slice(start, end + 1) });
		from = end;
	}
	return tags;
}

function deadButtons(file: string): string[] {
	const source = readFileSync(file, 'utf8').replace(/<!--[\s\S]*?-->/g, (c) =>
		c.replace(/[^\n]/g, ' '),
	);
	const name = relative(root, file);
	const line = (index: number) => source.slice(0, index).split('\n').length;
	const found: string[] = [];
	for (const match of source.matchAll(/<label\b[\s\S]*?<\/label>/g))
		if (/<(Button|button)\b/.test(match[0]))
			found.push(`${name}:${line(match.index)} button inside <label>`);
	if (ALLOWED.has(name)) return found;
	for (const element of ['Button', 'button'])
		for (const { index, tag } of openingTags(source, element)) {
			if (ACTION.test(tag)) continue;
			const inForm = source.lastIndexOf('<form', index) > source.lastIndexOf('</form>', index);
			if (element === 'button' && inForm && !tag.includes('type="button"')) continue;
			if (/(Trigger|Close|Action|Cancel)\b[^<]*>\s*$/.test(source.slice(index - 300, index)))
				continue;
			found.push(`${name}:${line(index)} ${element} with no action`);
		}
	return found;
}

it('every button does something', () => {
	expect(svelteFiles(root).flatMap(deadButtons)).toEqual([]);
});
