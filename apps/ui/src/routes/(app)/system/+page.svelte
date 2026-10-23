<script lang="ts">
	import { brandedTitle } from '#lib/ee/branding/branding.svelte.js';
	import { onMount } from 'svelte';
	import { page } from '$app/state';
	import { goto } from '$app/navigation';
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import SectionNav from '#lib/components/custom/section-nav.svelte';
	import SystemInfoPanel from '#lib/components/system/system-info-panel.svelte';
	import ContainersPanel from '#lib/components/system/containers-panel.svelte';
	import OrganizationIcon from '#lib/components/organization-icon.svelte';
	import { listOrganizations, type OrganizationDto } from '#lib/clients/organization-client.js';
	import CaddyPanel from '#lib/components/system/caddy-panel.svelte';
	import MaintenancePanel from '#lib/components/system/maintenance-panel.svelte';
	import BackupsPanel from '#lib/components/system/backups-panel.svelte';
	import InstanceSettingsPanel from '#lib/components/system/instance-settings-panel.svelte';
	import BuildsPanel from '#lib/components/system/builds-panel.svelte';
	import RateLimitPanel from '#lib/components/system/rate-limit-panel.svelte';
	import BrandingPanel from '#lib/ee/branding/branding-panel.svelte';
	import CertificatesSettings from '#lib/components/custom/certificates-settings.svelte';
	import SystemStatusSection from '#lib/components/home/system-status-section.svelte';
	import { getMe, getRegistrationStatus } from '#lib/clients/auth-client.js';
	import { getSelectedOrganizationId } from '#lib/utils/selected-organization.js';
	import { toast } from '#lib/utils/toast.js';
	import { tabIndexFromKeydown } from '#lib/utils/tab-shortcut.js';
	import InfoIcon from '@lucide/svelte/icons/info';
	import ContainerIcon from '@lucide/svelte/icons/container';
	import NetworkIcon from '@lucide/svelte/icons/network';
	import ServerCogIcon from '@lucide/svelte/icons/server-cog';
	import BugIcon from '@lucide/svelte/icons/bug';
	import DatabaseBackupIcon from '@lucide/svelte/icons/database-backup';
	import SlidersHorizontalIcon from '@lucide/svelte/icons/sliders-horizontal';
	import ShieldCheckIcon from '@lucide/svelte/icons/shield-check';
	import CpuIcon from '@lucide/svelte/icons/cpu';
	import GaugeIcon from '@lucide/svelte/icons/gauge';
	import PaletteIcon from '@lucide/svelte/icons/palette';
	import { t } from '#lib/i18n/index.svelte.js';

	/** Every section `?tab=` may name; anything else lands on System info. */
	const TABS = [
		'info',
		'general',
		'certificates',
		'builds',
		'rate-limit',
		'branding',
		'containers',
		'caddy',
		'maintenance',
		'debug',
		'backups',
	];

	// Containers and Caddy act on one organization's servers: the one the switcher selected.
	// The header switcher reloads the page, so this is read once per org.
	const organizationId = getSelectedOrganizationId();
	let organization = $state<OrganizationDto | null>(null);
	if (organizationId)
		listOrganizations()
			.then((all) => (organization = all.find((o) => o.id === organizationId) ?? null))
			.catch(() => {});

	let authorized = $state(false);
	// White-label is an enterprise, self-hosted feature: cloud has no endpoint for it.
	let isCloud = $state(true);
	let activeTab = $state(tabFromUrl());

	const tabItems = $derived([
		{ value: 'info', label: t('menu.user_menu.system_info'), icon: InfoIcon },
		{ value: 'general', label: t('account.settings.tabs.general'), icon: SlidersHorizontalIcon },
		{ value: 'certificates', label: t('account.certificates.title'), icon: ShieldCheckIcon },
		{ value: 'builds', label: t('account.settings.tabs.builds'), icon: CpuIcon },
		{ value: 'rate-limit', label: t('account.settings.tabs.rate_limit'), icon: GaugeIcon },
		...(isCloud
			? []
			: [{ value: 'branding', label: t('account.settings.tabs.branding'), icon: PaletteIcon }]),
		...(organizationId
			? [
					{ value: 'containers', label: t('menu.navigation.containers'), icon: ContainerIcon },
					{ value: 'caddy', label: t('menu.navigation.caddy'), icon: NetworkIcon },
				]
			: []),
		{ value: 'maintenance', label: t('menu.user_menu.maintenance'), icon: ServerCogIcon },
		{ value: 'debug', label: t('menu.user_menu.debug'), icon: BugIcon },
		{ value: 'backups', label: t('menu.navigation.backups'), icon: DatabaseBackupIcon },
	]);
	const visibleTab = $derived(
		tabItems.some((item) => item.value === activeTab) ? activeTab : 'info',
	);

	function tabFromUrl() {
		const tab = page.url.searchParams.get('tab');
		return tab && TABS.includes(tab) ? tab : 'info';
	}

	function setTab(value: string) {
		activeTab = value;
		// Panel-level params (`server`, `view`) belong to the section being left.
		const url = new URL(page.url.origin + page.url.pathname);
		url.searchParams.set('tab', value);
		goto(url, { shallow: true, replace: true });
	}

	function handleTabShortcut(event: KeyboardEvent) {
		const index = tabIndexFromKeydown(event, tabItems.length);
		if (index === null) return;
		event.preventDefault();
		setTab(tabItems[index].value);
	}

	onMount(async () => {
		try {
			const me = await getMe();
			if (!me.is_sysadmin) {
				toast.error(t('operations.system.access_required'));
				goto('/home');
				return;
			}
		} catch {
			goto('/login');
			return;
		}
		isCloud = await getRegistrationStatus().then(
			(status) => status.is_cloud,
			() => true,
		);
		authorized = true;
	});
</script>

<svelte:head>
	<title>{brandedTitle(t('operations.system.system'))}</title>
</svelte:head>

<svelte:window onkeydown={handleTabShortcut} />

<div class="flex w-full min-w-0 flex-col gap-5 px-4 py-6 sm:px-6">
	<div class="min-w-0">
		<h1 class="text-xl font-semibold tracking-tight">{t('operations.system.system')}</h1>
		<p class="text-sm text-muted-foreground">{t('operations.system.hub_description')}</p>
	</div>

	{#if !authorized}
		<div class="flex flex-col gap-6 lg:flex-row" aria-busy="true">
			<div class="flex gap-2 lg:w-52 lg:flex-col">
				{#each { length: 5 } as _, index (index)}
					<Skeleton class="h-8 w-24 rounded-lg lg:w-full" />
				{/each}
			</div>
			<div class="flex flex-1 flex-col gap-3">
				<Skeleton class="h-6 w-40" />
				<Skeleton class="h-40 w-full rounded-2xl" />
			</div>
		</div>
	{:else}
		<div class="flex flex-col gap-6 lg:flex-row lg:items-start">
			<SectionNav
				items={tabItems}
				current={visibleTab}
				label={t('operations.system.system')}
				onselect={setTab} />

			<!-- Only the visible section is mounted: several of them poll or stream. -->
			<div class="flex min-w-0 flex-1 flex-col gap-4">
				{#if (visibleTab === 'containers' || visibleTab === 'caddy') && organization}
					<p class="flex items-center gap-2 text-sm text-muted-foreground">
						{t('operations.system.org_scope')}
						<span
							class="lp-raised inline-flex h-6 items-center gap-1.5 rounded-md bg-muted px-2 text-xs font-medium text-foreground">
							<OrganizationIcon
								icon={organization.icon}
								bg={organization.icon_bg}
								class="size-3.5" />
							{organization.name}
						</span>
					</p>
				{/if}
				{#if visibleTab === 'containers' && organizationId}
					<ContainersPanel {organizationId} />
				{:else if visibleTab === 'caddy' && organizationId}
					<CaddyPanel {organizationId} />
				{:else if visibleTab === 'general'}
					<InstanceSettingsPanel />
				{:else if visibleTab === 'certificates'}
					<CertificatesSettings />
				{:else if visibleTab === 'builds'}
					<BuildsPanel />
				{:else if visibleTab === 'rate-limit'}
					<RateLimitPanel />
				{:else if visibleTab === 'branding'}
					<BrandingPanel />
				{:else if visibleTab === 'maintenance'}
					<MaintenancePanel />
				{:else if visibleTab === 'debug'}
					<div>
						<h2 class="text-lg font-semibold tracking-tight">{t('operations.debug.title')}</h2>
						<p class="mt-1 text-sm text-muted-foreground">{t('operations.debug.description')}</p>
					</div>
					<SystemStatusSection showDatabase />
				{:else if visibleTab === 'backups'}
					<BackupsPanel />
				{:else}
					<SystemInfoPanel />
				{/if}
			</div>
		</div>
	{/if}
</div>
