import { parse } from 'smol-toml';

const root = new URL('../src/lib/i18n/messages/', import.meta.url).pathname;
const catalogs: Record<string, Record<string, string>> = { en: {}, es: {} };

function flatten(value: unknown, prefix: string, target: Record<string, string>) {
	if (typeof value === 'string') return void (target[prefix] = value);
	if (!value || typeof value !== 'object' || Array.isArray(value)) throw new Error(`${prefix} must be a string or table`);
	for (const [key, child] of Object.entries(value)) flatten(child, `${prefix}.${key}`, target);
}

for await (const file of new Bun.Glob('*/*.toml').scan({ cwd: root })) {
	const [, locale, domain] = file.match(/^(en|es)\/([^/]+)\.toml$/) ?? [];
	if (!locale || !domain) continue;
	for (const [key, value] of Object.entries(parse(await Bun.file(`${root}${file}`).text()))) {
		flatten(value, `${domain}.${key}`, catalogs[locale]);
	}
}

let failed = false;
for (const key of new Set([...Object.keys(catalogs.en), ...Object.keys(catalogs.es)])) {
	if (!(key in catalogs.en)) console.warn(`WARNING missing-key en ${key}`);
	else if (!(key in catalogs.es)) console.warn(`WARNING missing-key es ${key}`);
	else {
		const vars = (value: string) => [...value.matchAll(/\{([A-Za-z_][A-Za-z0-9_]*)\}/g)].map((match) => match[1]).sort().join(',');
		if (vars(catalogs.en[key]) !== vars(catalogs.es[key])) {
			console.error(`ERROR placeholder-mismatch ${key}`);
			failed = true;
		}
	}
}
if (failed) process.exit(1);
