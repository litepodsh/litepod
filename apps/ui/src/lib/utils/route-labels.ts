import type { Component } from 'svelte';
import AccountIcon from '@lucide/svelte/icons/user';
import AddIcon from '@lucide/svelte/icons/plus';
import AdminIcon from '@lucide/svelte/icons/shield-check';
import ApplicationsIcon from '@lucide/svelte/icons/boxes';
import BackupsIcon from '@lucide/svelte/icons/database-backup';
import BillingIcon from '@lucide/svelte/icons/credit-card';
import BuildsIcon from '@lucide/svelte/icons/wrench';
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
import PodmanIcon from '@lucide/svelte/icons/container';
import ProjectsIcon from '@lucide/svelte/icons/folder-kanban';
import ScheduledTasksIcon from '@lucide/svelte/icons/calendar-clock';
import ServersIcon from '@lucide/svelte/icons/server';
import SettingsIcon from '@lucide/svelte/icons/settings';
import SubscriptionIcon from '@lucide/svelte/icons/credit-card';
import SystemIcon from '@lucide/svelte/icons/container';
import TopologyIcon from '@lucide/svelte/icons/waypoints';
import UsersIcon from '@lucide/svelte/icons/users';

/**
 * Human labels for URL segments, shared by the header breadcrumb and the
 * document title so the two can never drift apart.
 */
export const ROUTE_LABELS: Record<string, string> = {
	account: 'Account',
	add: 'Create organization',
	admin: 'Admin',
	applications: 'Applications',
	backups: 'Backups',
	billing: 'Billing',
	builds: 'Build configuration',
	caddy: 'Caddy',
	containers: 'Containers',
	debug: 'Debug',
	'deployment-queue': 'Deployment queue',
	environments: 'Environments',
	'git-providers': 'Git providers',
	home: 'Home',
	maintenance: 'Maintenance',
	notifications: 'Notifications',
	organizations: 'Organizations',
	podman: 'Podman',
	projects: 'Projects',
	'scheduled-tasks': 'Scheduled Tasks',
	servers: 'Servers',
	settings: 'Settings',
	subscription: 'Subscription',
	system: 'System',
	topology: 'Map',
	users: 'Users',
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
	builds: BuildsIcon,
	caddy: CaddyIcon,
	containers: ContainersIcon,
	debug: DebugIcon,
	'deployment-queue': DeploymentQueueIcon,
	environments: EnvironmentsIcon,
	'git-providers': GitProvidersIcon,
	home: HomeIcon,
	maintenance: MaintenanceIcon,
	notifications: NotificationsIcon,
	organizations: OrganizationsIcon,
	podman: PodmanIcon,
	projects: ProjectsIcon,
	'scheduled-tasks': ScheduledTasksIcon,
	servers: ServersIcon,
	settings: SettingsIcon,
	subscription: SubscriptionIcon,
	system: SystemIcon,
	topology: TopologyIcon,
	users: UsersIcon,
};
