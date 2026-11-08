import { describe, expect, it } from 'vitest';
import { formatVolumeFile } from './volume-file-format.js';

describe('formatVolumeFile', () => {
	it('formats nested and self-closing XML elements', async () => {
		const formatted = await formatVolumeFile('nested.xml', '<root><item/><item>text</item></root>');

		expect(formatted).toContain('<root>\n');
		expect(formatted).toContain('  <item />\n');
		expect(formatted).toContain('  <item>text</item>\n');
		expect(formatted).toContain('</root>');
	});

	it('preserves XML text whitespace while formatting nested elements', async () => {
		const formatted = await formatVolumeFile('text.xml', '<root><item> A   B </item></root>');

		expect(formatted).toContain('  <item> A   B </item>');
	});

	it('preserves XML namespaces and attribute values', async () => {
		const formatted = await formatVolumeFile(
			'namespaced.xml',
			'<root xmlns:x="urn:example"><x:item id="a" enabled="true"/></root>',
		);

		expect(formatted).toContain('xmlns:x="urn:example"');
		expect(formatted).toContain('x:item id="a" enabled="true"');
	});

	it('rejects malformed XML', async () => {
		await expect(formatVolumeFile('broken.xml', '<root><item></root>')).rejects.toThrow();
	});

	it('continues formatting JSON files', async () => {
		const formatted = await formatVolumeFile('data.json', '{"value":1}');

		expect(formatted).toBe('{ "value": 1 }\n');
	});

	it('returns null for unsupported extensions', async () => {
		await expect(formatVolumeFile('notes.txt', 'plain text')).resolves.toBeNull();
	});
});
