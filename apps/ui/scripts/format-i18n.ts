import { parse, stringify } from 'smol-toml';

const root = new URL('../src/lib/i18n/messages/', import.meta.url).pathname;
const check = Bun.argv.includes('--check');

function sort(value: unknown): unknown {
	if (!value || typeof value !== 'object' || Array.isArray(value)) return value;
	const entries = Object.entries(value as Record<string, unknown>).sort(([left, leftValue], [right, rightValue]) => {
		const leftTable = leftValue !== null && typeof leftValue === 'object' && !Array.isArray(leftValue);
		const rightTable = rightValue !== null && typeof rightValue === 'object' && !Array.isArray(rightValue);
		return leftTable === rightTable ? left.localeCompare(right) : leftTable ? 1 : -1;
	});
	return Object.fromEntries(entries.map(([key, child]) => [key, sort(child)]));
}

let changed = false;
for await (const file of new Bun.Glob('*/*.toml').scan({ cwd: root })) {
	const path = `${root}${file}`;
	const source = await Bun.file(path).text();
	const formatted = stringify(sort(parse(source)) as Record<string, unknown>);
	if (source === formatted) continue;
	changed = true;
	if (check) console.error(`not formatted: ${file}`);
	else await Bun.write(path, formatted);
}

if (check && changed) process.exit(1);
