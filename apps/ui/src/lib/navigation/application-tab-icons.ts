import type { Component } from 'svelte';
import LayoutDashboardIcon from '@lucide/svelte/icons/layout-dashboard';
import DatabaseBackupIcon from '@lucide/svelte/icons/database-backup';
import KeyRoundIcon from '@lucide/svelte/icons/key-round';
import GlobeIcon from '@lucide/svelte/icons/globe';
import RocketIcon from '@lucide/svelte/icons/rocket';
import NetworkIcon from '@lucide/svelte/icons/network';
import SquareTerminalIcon from '@lucide/svelte/icons/square-terminal';
import ScrollTextIcon from '@lucide/svelte/icons/scroll-text';
import SlidersHorizontalIcon from '@lucide/svelte/icons/sliders-horizontal';
import ChartColumnIcon from '@lucide/svelte/icons/chart-column';
import CalendarClockIcon from '@lucide/svelte/icons/calendar-clock';

/** Icons for the application page's `?tab=` values, shown in the tab capsule. */
export const APPLICATION_TAB_ICONS: Record<string, Component> = {
	general: LayoutDashboardIcon,
	backups: DatabaseBackupIcon,
	environment: KeyRoundIcon,
	domains: GlobeIcon,
	deployment: RocketIcon,
	containers: NetworkIcon,
	terminal: SquareTerminalIcon,
	logs: ScrollTextIcon,
	analytics: ChartColumnIcon,
	tasks: CalendarClockIcon,
	advanced: SlidersHorizontalIcon,
};
