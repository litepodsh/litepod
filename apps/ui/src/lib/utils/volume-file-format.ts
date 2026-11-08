const parserByExtension: Record<string, string> = {
	json: 'json',
	jsonc: 'json',
	js: 'babel',
	mjs: 'babel',
	cjs: 'babel',
	ts: 'typescript',
	tsx: 'typescript',
	html: 'html',
	htm: 'html',
	svg: 'html',
	xml: 'xml',
	css: 'css',
	scss: 'css',
	less: 'css',
	md: 'markdown',
	mdx: 'mdx',
	yml: 'yaml',
	yaml: 'yaml',
};

export async function formatVolumeFile(path: string, source: string): Promise<string | null> {
	const extension = path.split('.').pop()?.toLowerCase();
	const parser = parserByExtension[extension ?? ''];
	if (!parser) return null;

	const prettier = await import('prettier/standalone');
	const [babel, estree, html, markdown, postcss, typescript, yaml, xml] = await Promise.all([
		import('prettier/plugins/babel'),
		import('prettier/plugins/estree'),
		import('prettier/plugins/html'),
		import('prettier/plugins/markdown'),
		import('prettier/plugins/postcss'),
		import('prettier/plugins/typescript'),
		import('prettier/plugins/yaml'),
		parser === 'xml' ? import('@prettier/plugin-xml') : Promise.resolve(null),
	]);

	return prettier.format(source, {
		parser,
		plugins: [
			babel.default,
			estree.default,
			html.default,
			markdown.default,
			postcss.default,
			typescript.default,
			yaml.default,
			...(xml ? [xml.default] : []),
		],
		...(parser === 'xml' ? { xmlWhitespaceSensitivity: 'preserve' as const } : {}),
	});
}
