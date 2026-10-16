import { describe, expect, it } from 'vitest';

import { hasDeprecatedComposeVersion } from './compose-version';

describe('hasDeprecatedComposeVersion', () => {
	it('detects version with double quotes', () => {
		const result = hasDeprecatedComposeVersion('version: "3.0"');
		expect(result).toMatchObject({ found: true, line: 1, version: '3.0' });
	});

	it('detects version with single quotes', () => {
		const result = hasDeprecatedComposeVersion("version: '3'");
		expect(result).toMatchObject({ found: true, version: '3' });
	});

	it('detects version without quotes', () => {
		const result = hasDeprecatedComposeVersion('version: 2.0');
		expect(result).toMatchObject({ found: true, version: '2.0' });
	});

	it('does not match indented version (not top-level)', () => {
		const result = hasDeprecatedComposeVersion('  version: "3.0"');
		expect(result.found).toBe(false);
	});

	it('reports 1-based line and column', () => {
		const result = hasDeprecatedComposeVersion('services:\n  app:\nversion: "3.0"');
		expect(result).toMatchObject({ found: true, line: 3, column: 1, version: '3.0' });
	});

	it('does not match indented version under services', () => {
		const content = `services:
  app:
    version: "3.0"`;
		const result = hasDeprecatedComposeVersion(content);
		expect(result.found).toBe(false);
	});

	it('does not match commented-out version', () => {
		const result = hasDeprecatedComposeVersion('# version: "3.0"');
		expect(result.found).toBe(false);
	});

	it('returns not found for empty content', () => {
		const result = hasDeprecatedComposeVersion('');
		expect(result).toMatchObject({ found: false, version: '' });
	});

	it('returns not found when no version field exists', () => {
		const content = `services:
  app:
    image: nginx:latest`;
		const result = hasDeprecatedComposeVersion(content);
		expect(result.found).toBe(false);
	});

	it('does not match tab-indented version (not top-level)', () => {
		const result = hasDeprecatedComposeVersion('\tversion: "3.0"');
		expect(result.found).toBe(false);
	});

	it('handles version with extra spaces around colon', () => {
		const result = hasDeprecatedComposeVersion('version : "3.0"');
		expect(result).toMatchObject({ found: true, version: '3.0' });
	});

	it('captures the length of the whole match', () => {
		const result = hasDeprecatedComposeVersion('version: "3.0"');
		expect(result.length).toBe(result.found ? 'version: "3.0"'.length : 0);
	});
});
