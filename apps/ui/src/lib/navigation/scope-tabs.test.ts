import { expect, it } from 'vitest';
import { activeTab, orgTabs, scopeOf } from './scope-tabs';

it('detects the scope from the path', () => {
	expect(scopeOf('/home')).toBe('org');
	expect(scopeOf('/projects')).toBe('org');
	expect(scopeOf('/organizations/org1/servers')).toBe('org');
	expect(scopeOf('/projects/p1/environments/e1')).toBe('project');
	expect(scopeOf('/projects/p1/environments/e1/applications/a1')).toBe('app');
	expect(scopeOf('/account')).toBe('none');
	for (const path of ['/system', '/maintenance', '/debug', '/settings/backups'])
		expect(scopeOf(path)).toBe('org');
	expect(scopeOf('/settings')).toBe('none');
});

it('builds org tabs on the existing routes', () => {
	const tabs = orgTabs('org1');
	expect(tabs.map((tab) => tab.id)).toEqual([
		'home',
		'projects',
		'apps',
		'deployments',
		'domains',
		'analytics',
		'servers',
		'tasks',
		'settings',
	]);
	expect(tabs.find((tab) => tab.id === 'tasks')!.href).toBe('/organizations/org1/scheduled-tasks');
});

it('adds the System tab last for sysadmins only', () => {
	expect(orgTabs('org1').some((tab) => tab.id === 'system')).toBe(false);
	const tabs = orgTabs('org1', true);
	expect(tabs.at(-1)).toMatchObject({ id: 'system', href: '/system' });
	for (const path of ['/system', '/maintenance', '/debug', '/settings/backups'])
		expect(activeTab(path, tabs)).toBe('system');
});

it('adds Billing for cloud customers only', () => {
	expect(orgTabs('org1').some((tab) => tab.id === 'billing')).toBe(false);
	expect(orgTabs('org1', true, true).some((tab) => tab.id === 'billing')).toBe(false);
	const tabs = orgTabs('org1', false, true);
	expect(tabs.find((tab) => tab.id === 'billing')!.href).toBe('/billing');
	expect(scopeOf('/billing')).toBe('org');
	expect(activeTab('/billing', tabs)).toBe('billing');
});

it('marks the tab that owns the current page', () => {
	const tabs = orgTabs('org1');
	expect(activeTab('/home', tabs)).toBe('home');
	expect(activeTab('/organizations/org1/topology', tabs)).toBe('servers');
	expect(activeTab('/organizations/org1', tabs)).toBe('settings');
	expect(activeTab('/git-providers', tabs)).toBe('settings');
	expect(activeTab('/account', tabs)).toBeUndefined();
});
