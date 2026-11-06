import type { Component } from 'svelte';
import HouseIcon from '@lucide/svelte/icons/house';
import FolderKanbanIcon from '@lucide/svelte/icons/folder-kanban';
import RocketIcon from '@lucide/svelte/icons/rocket';
import BoxesIcon from '@lucide/svelte/icons/boxes';
import GlobeIcon from '@lucide/svelte/icons/globe';
import ChartColumnIcon from '@lucide/svelte/icons/chart-column';
import ServerIcon from '@lucide/svelte/icons/server';
import CalendarClockIcon from '@lucide/svelte/icons/calendar-clock';
import PlugIcon from '@lucide/svelte/icons/plug';
import SettingsIcon from '@lucide/svelte/icons/settings';
import ServerCogIcon from '@lucide/svelte/icons/server-cog';
import CreditCardIcon from '@lucide/svelte/icons/credit-card';
import ShieldCheckIcon from '@lucide/svelte/icons/shield-check';

/**
 * Which level of the console a path belongs to. Organization tabs come from
 * this registry; project and application pages push their own tabs (they
 * depend on permissions and the app's template), see `shell.svelte.ts`.
 */
export type Scope = 'org' | 'project' | 'app' | 'none';

export type ScopeTab = {
	id: string;
	labelKey: string;
	icon: Component;
	href: string;
	/** Path prefixes this tab owns besides its own href. */
	owns?: string[];
	/** Only its own href (and `owns`) count, not every path under it. */
	exact?: boolean;
};

const ORG_PATHS = [
	/^\/home$/,
	/^\/projects$/,
	/^\/organizations(\/|$)/,
	/^\/git-providers/,
	/^\/notifications/,
	/^\/system$/,
	/^\/maintenance$/,
	/^\/debug$/,
	/^\/settings\/backups$/,
	/^\/billing$/,
	/^\/admin(\/|$)/,
];

export function scopeOf(pathname: string): Scope {
	if (/^\/projects\/[^/]+\/environments\/[^/]+\/applications\/[^/]+/.test(pathname)) return 'app';
	if (/^\/projects\/[^/]+/.test(pathname)) return 'project';
	return ORG_PATHS.some((re) => re.test(pathname)) ? 'org' : 'none';
}

/**
 * `isSysadmin` adds the instance-wide System hub at the end. `isCloud` adds
 * Billing for cloud customers; sysadmins have no subscription.
 *
 * The Admin portal is the cloud edition's counterpart to the System hub: only
 * the SysAdmin of a cloud instance gets one, and the API enforces the same
 * rule on the routes behind it (`/api/v1/admin/*`).
 */
export function orgTabs(organizationId: string, isSysadmin = false, isCloud = false): ScopeTab[] {
	const base = `/organizations/${encodeURIComponent(organizationId)}`;
	return [
		{ id: 'home', labelKey: 'menu.tabs.home', icon: HouseIcon, href: '/home' },
		{ id: 'projects', labelKey: 'menu.tabs.projects', icon: FolderKanbanIcon, href: '/projects' },
		{ id: 'apps', labelKey: 'menu.tabs.apps', icon: BoxesIcon, href: `${base}/apps` },
		{
			id: 'deployments',
			labelKey: 'menu.tabs.deployments',
			icon: RocketIcon,
			href: `${base}/deployment-queue`,
		},
		{ id: 'domains', labelKey: 'menu.tabs.domains', icon: GlobeIcon, href: `${base}/domains` },
		{
			id: 'analytics',
			labelKey: 'menu.tabs.analytics',
			icon: ChartColumnIcon,
			href: `${base}/analytics`,
		},
		{
			id: 'servers',
			labelKey: 'menu.tabs.servers',
			icon: ServerIcon,
			href: `${base}/servers`,
			owns: [`${base}/server-groups`, `${base}/topology`],
		},
		{
			id: 'tasks',
			labelKey: 'menu.tabs.scheduled_tasks',
			icon: CalendarClockIcon,
			href: `${base}/scheduled-tasks`,
		},
		...(isCloud && !isSysadmin
			? [{ id: 'billing', labelKey: 'menu.tabs.billing', icon: CreditCardIcon, href: '/billing' }]
			: []),
		{
			id: 'integrations',
			labelKey: 'menu.tabs.integrations',
			icon: PlugIcon,
			href: `${base}/integrations`,
			owns: ['/git-providers', '/notifications'],
		},
		{
			id: 'settings',
			labelKey: 'menu.tabs.settings',
			icon: SettingsIcon,
			href: base,
			exact: true,
		},
		...(isSysadmin
			? [
					{
						id: 'system',
						labelKey: 'menu.tabs.system',
						icon: ServerCogIcon,
						href: '/system',
						owns: [
							'/maintenance',
							'/debug',
							'/settings/backups',
							`${base}/containers`,
							`${base}/caddy`,
						],
					},
				]
			: []),
		...(isSysadmin && isCloud
			? [
					{
						id: 'admin',
						labelKey: 'menu.tabs.admin',
						icon: ShieldCheckIcon,
						href: '/admin',
						owns: ['/admin/users', '/admin/security', '/admin/billing'],
					},
				]
			: []),
	];
}

export function activeTab(pathname: string, tabs: ScopeTab[]): string | undefined {
	const under = (prefix: string) => pathname === prefix || pathname.startsWith(`${prefix}/`);
	return (
		tabs.find((tab) => pathname === tab.href)?.id ??
		tabs.find((tab) => tab.owns?.some(under))?.id ??
		tabs.find((tab) => !tab.exact && under(tab.href))?.id
	);
}
