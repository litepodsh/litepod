import { parse } from 'smol-toml';

export type Locale = 'en' | 'es';
export type MessageMap = Readonly<Record<string, string>>;
export type Catalogs = Readonly<Record<Locale, MessageMap>>;
export type CatalogIssue =
	| { level: 'warning'; code: 'missing-key'; locale: Locale; key: string }
	| { level: 'error'; code: 'placeholder-mismatch'; key: string };

function flatten(value: unknown, prefix: string, messages: Record<string, string>) {
	if (typeof value === 'string') return void (messages[prefix] = value);
	if (!value || typeof value !== 'object' || Array.isArray(value)) {
		throw new TypeError(`${prefix} must be a string or TOML table`);
	}
	for (const [key, child] of Object.entries(value)) flatten(child, `${prefix}.${key}`, messages);
}

export function parseCatalog(source: string, namespace: string): MessageMap {
	const messages: Record<string, string> = {};
	for (const [key, value] of Object.entries(parse(source)))
		flatten(value, `${namespace}.${key}`, messages);
	return messages;
}

function mergeCatalogs(maps: MessageMap[]): MessageMap {
	const merged: Record<string, string> = {};
	for (const map of maps)
		for (const [key, value] of Object.entries(map)) {
			if (key in merged) throw new Error(`Duplicate i18n key: ${key}`);
			merged[key] = value;
		}
	return merged;
}

const rawCatalogs = import.meta.glob('./messages/*/*.toml', {
	eager: true,
	query: '?raw',
	import: 'default',
}) as Record<string, string>;
const byLocale: Record<Locale, MessageMap[]> = { en: [], es: [] };
for (const [path, source] of Object.entries(rawCatalogs)) {
	const match = path.match(/^\.\/messages\/(en|es)\/([^/]+)\.toml$/);
	if (match) byLocale[match[1] as Locale].push(parseCatalog(source, match[2]));
}
export const catalogs: Catalogs = {
	en: mergeCatalogs(byLocale.en),
	es: mergeCatalogs(byLocale.es),
};

function placeholders(message: string) {
	return [...message.matchAll(/\{([A-Za-z_][A-Za-z0-9_]*)\}/g)]
		.map((match) => match[1])
		.sort()
		.join('\0');
}

export function validateCatalogs(source: Catalogs): CatalogIssue[] {
	const issues: CatalogIssue[] = [];
	for (const key of new Set([...Object.keys(source.en), ...Object.keys(source.es)])) {
		const en = source.en[key];
		const es = source.es[key];
		if (en === undefined) issues.push({ level: 'warning', code: 'missing-key', locale: 'en', key });
		else if (es === undefined)
			issues.push({ level: 'warning', code: 'missing-key', locale: 'es', key });
		else if (placeholders(en) !== placeholders(es))
			issues.push({ level: 'error', code: 'placeholder-mismatch', key });
	}
	return issues;
}
