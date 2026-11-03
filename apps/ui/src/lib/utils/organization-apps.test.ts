import { describe, expect, it } from 'vitest';
import {
	filterApps,
	primaryDomain,
	statusTone,
	type OrganizationApp,
	appBoardGroups,
} from './organization-apps';
import type { AppSummaryDomainDto } from '#lib/clients/organization-client.js';

const domain = (host: string, is_primary = false) => ({ host, is_primary }) as AppSummaryDomainDto;
const app = (name: string, project: string, updated_at: number, hosts: string[] = []) =>
	({
		application: { id: name, name, updated_at },
		project: { id: project, name: project },
		environment: { id: 'e', name: 'prod', is_production: true },
		status: 'running',
		domains: hosts.map((host) => domain(host)),
		href: `/x/${name}`,
	}) as unknown as OrganizationApp;

const apps = [
	app('ente', 'tools', 1, ['photos.sebasgc.xyz']),
	app('umami', 'tools', 3),
	app('blog', 'web', 2),
];

it('sorts newest first and filters by project', () => {
	expect(filterApps(apps, '').map((a) => a.application.name)).toEqual(['umami', 'blog', 'ente']);
	expect(filterApps(apps, '', 'web').map((a) => a.application.name)).toEqual(['blog']);
});

it('searches names, projects and domains', () => {
	expect(filterApps(apps, 'PHOTOS').map((a) => a.application.name)).toEqual(['ente']);
	expect(filterApps(apps, 'web').map((a) => a.application.name)).toEqual(['blog']);
});

it('prefers the primary domain', () => {
	expect(primaryDomain([domain('a'), domain('b', true)])?.host).toBe('b');
	expect(primaryDomain([])).toBeUndefined();
});

it('maps deployment status onto the shared tones', () => {
	expect(statusTone('running')).toBe('ok');
	expect(statusTone('failed')).toBe('danger');
	expect(statusTone(null)).toBe('neutral');
});

describe('appBoardGroups', () => {
	const board = (name: string, status: string | null, updated_at = 1) =>
		({ application: { name, updated_at }, project: { name: 'p' }, status, domains: [] }) as never;
	const shape = (groups: ReturnType<typeof appBoardGroups>) =>
		groups.map((group) => [group.key, group.apps.map((app) => app.application.name)]);

	it('orders groups by urgency and drops empty ones', () => {
		const groups = appBoardGroups(
			[board('a', 'running'), board('b', 'failed'), board('c', 'stopped'), board('d', null)],
			10,
		);
		expect(shape(groups)).toEqual([
			['attention', ['b']],
			['running', ['a']],
			['other', ['c', 'd']],
		]);
	});

	it('never cuts failed or deploying apps, and trims running before the rest', () => {
		const groups = appBoardGroups(
			[
				board('f1', 'failed'),
				board('f2', 'failed'),
				board('d1', 'deploying'),
				board('r1', 'running'),
				board('s1', 'stopped'),
			],
			2,
		);
		expect(shape(groups)).toEqual([
			['attention', ['f1', 'f2']],
			['progress', ['d1']],
		]);
	});
});
