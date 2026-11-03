import type { Component } from 'svelte';
import IntegrationsIcon from '@lucide/svelte/icons/plug';
import AccountIcon from '@lucide/svelte/icons/user';
import AddIcon from '@lucide/svelte/icons/plus';
import AdminIcon from '@lucide/svelte/icons/shield-check';
import ApplicationsIcon from '@lucide/svelte/icons/boxes';
import BackupsIcon from '@lucide/svelte/icons/database-backup';
import BillingIcon from '@lucide/svelte/icons/credit-card';
import CaddyIcon from '@lucide/svelte/icons/network';
import ContainersIcon from '@lucide/svelte/icons/container';
import DebugIcon from '@lucide/svelte/icons/bug';
import DeploymentQueueIcon from '@lucide/svelte/icons/list-todo';
import EnvironmentsIcon from '@lucide/svelte/icons/layers';
import GitProvidersIcon from '@lucide/svelte/icons/git-branch';
import HomeIcon from '@lucide/svelte/icons/house';
import MaintenanceIcon from '@lucide/svelte/icons/server-cog';
import NotificationsIcon from '@lucide/svelte/icons/bell';
import OrganizationsIcon from '@lucide/svelte/icons/building-2';
import ProjectsIcon from '@lucide/svelte/icons/folder-kanban';
import ScheduledTasksIcon from '@lucide/svelte/icons/calendar-clock';
import ServersIcon from '@lucide/svelte/icons/server';
import SettingsIcon from '@lucide/svelte/icons/settings';
import SubscriptionIcon from '@lucide/svelte/icons/credit-card';
import SystemIcon from '@lucide/svelte/icons/container';
import TopologyIcon from '@lucide/svelte/icons/waypoints';
import UsersIcon from '@lucide/svelte/icons/users';

/**
 * Translation keys for URL segments, shared by the header breadcrumb and the
 * document title so the two can never drift apart.
 */
export const ROUTE_LABELS: Record<string, string> = {
	account: 'menu.user_menu.account',
	add: 'organizations.add.title',
	admin: 'menu.navigation.admin',
	analytics: 'menu.tabs.analytics',
	applications: 'projects.environment.applications',
	apps: 'menu.tabs.apps',
	backups: 'menu.navigation.backups',
	billing: 'menu.tabs.billing',
	caddy: 'menu.navigation.caddy',
	containers: 'menu.navigation.containers',
	debug: 'menu.user_menu.debug',
	'deployment-queue': 'menu.navigation.deployment_queue',
	domains: 'menu.tabs.domains',
	integrations: 'menu.tabs.integrations',
	environments: 'projects.environment.environments',
	'git-providers': 'menu.navigation.git_providers',
	home: 'menu.navigation.home',
	maintenance: 'menu.user_menu.maintenance',
	notifications: 'menu.navigation.notifications',
	organizations: 'menu.navigation.organizations',
	projects: 'menu.navigation.projects',
	'scheduled-tasks': 'menu.navigation.scheduled_tasks',
	security: 'operations.admin.security_title',
	'server-groups': 'menu.navigation.server_groups',
	servers: 'menu.navigation.servers',
	settings: 'menu.tabs.settings',
	subscription: 'menu.navigation.subscription',
	system: 'menu.navigation.system',
	topology: 'menu.navigation.map',
	users: 'operations.admin.users_title',
};

/**
 * Icons for URL segments, mirroring `ROUTE_LABELS` so the header breadcrumb can
 * render a leading glyph for every known route. Dynamic segments (ids) resolve
 * to `undefined` and are skipped.
 */
export const ROUTE_ICONS: Record<string, Component> = {
	account: AccountIcon,
	add: AddIcon,
	admin: AdminIcon,
	applications: ApplicationsIcon,
	backups: BackupsIcon,
	billing: BillingIcon,
	caddy: CaddyIcon,
	containers: ContainersIcon,
	debug: DebugIcon,
	'deployment-queue': DeploymentQueueIcon,
	environments: EnvironmentsIcon,
	'git-providers': GitProvidersIcon,
	home: HomeIcon,
	maintenance: MaintenanceIcon,
	notifications: NotificationsIcon,
	integrations: IntegrationsIcon,
	organizations: OrganizationsIcon,
	projects: ProjectsIcon,
	'scheduled-tasks': ScheduledTasksIcon,
	servers: ServersIcon,
	settings: SettingsIcon,
	subscription: SubscriptionIcon,
	system: SystemIcon,
	topology: TopologyIcon,
	users: UsersIcon,
};
