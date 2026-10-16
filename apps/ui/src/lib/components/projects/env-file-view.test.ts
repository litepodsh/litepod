import { render } from '@testing-library/svelte';
import { expect, it } from 'vitest';
import EnvFileView, { parseEnv } from './env-file-view.svelte';

const vars = { KEY: 'v', DATABASE_URL: 'postgres://u:p@h/db' };

it('masks values with line numbers and keys', () => {
	const { container } = render(EnvFileView, { vars, reveal: false });
	expect([...container.querySelectorAll('.ln')].map((n) => n.textContent)).toEqual(['1', '2']);
	expect([...container.querySelectorAll('.k')].map((n) => n.textContent)).toEqual([
		'KEY',
		'DATABASE_URL',
	]);
	expect(container.querySelectorAll('.m')).toHaveLength(2);
	expect(container.textContent).not.toContain('postgres://');
});

it('shows values when revealed', () => {
	const { container } = render(EnvFileView, { vars, reveal: true });
	expect([...container.querySelectorAll('.v')].map((n) => n.textContent)).toEqual([
		'v',
		'postgres://u:p@h/db',
	]);
	expect(container.querySelector('.m')).toBeNull();
});

it('marks values that contain a bound domain as linked', () => {
	const { container } = render(EnvFileView, {
		vars: { ...vars, APP_URL: 'https://app.example.test' },
		reveal: true,
		domains: ['app.example.test'],
	});
	const linked = [...container.querySelectorAll('.linked')].map((n) => n.textContent);
	expect(linked).toEqual(['https://app.example.test']);
});

it('reports the lines the API would reject', () => {
	expect(parseEnv('A=1\n\n# c\nlower=x\nNOEQ\nA=2\nB=a=b').errors).toEqual([4, 5, 6]);
	expect(parseEnv('B=a=b').vars).toEqual({ B: 'a=b' });
});
