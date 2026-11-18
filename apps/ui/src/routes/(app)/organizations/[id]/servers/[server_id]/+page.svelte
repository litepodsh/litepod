<script lang="ts">
	import { page } from '$app/state';
	import ActivityIcon from '@lucide/svelte/icons/activity';
	import ArrowLeftIcon from '@lucide/svelte/icons/arrow-left';
	import ContainerIcon from '@lucide/svelte/icons/container';
	import EllipsisVerticalIcon from '@lucide/svelte/icons/ellipsis-vertical';
	import ExternalLinkIcon from '@lucide/svelte/icons/external-link';
	import Maximize2Icon from '@lucide/svelte/icons/maximize-2';
	import PencilIcon from '@lucide/svelte/icons/pencil';
	import ShieldCheckIcon from '@lucide/svelte/icons/shield-check';
	import TerminalIcon from '@lucide/svelte/icons/terminal';
	import WrenchIcon from '@lucide/svelte/icons/wrench';
	import { AnimatedTabs } from '#lib/components/custom/animated-tabs/index.js';
	import CopyButton from '#lib/components/custom/copy-button.svelte';
	import MetricsHistoryExplorer from '#lib/components/metrics/metrics-history-explorer.svelte';
	import OsIcon from '#lib/components/os-icon.svelte';
	import RenameServerDialog from '#lib/components/servers/rename-server-dialog.svelte';
	import ServerHelperNotice from '#lib/components/servers/server-helper-notice.svelte';
	import ServerMaintenanceDialog from '#lib/components/servers/server-maintenance-dialog.svelte';
	import ServerSecurityDialog from '#lib/components/servers/server-security-dialog.svelte';
	import ServerSwapNotice from '#lib/components/servers/server-swap-notice.svelte';
	import ServerTerminalDialog from '#lib/components/servers/server-terminal-dialog.svelte';
	import ServerTerminalTab from '#lib/components/servers/server-terminal-tab.svelte';
	import ServerVulnerabilities from '#lib/components/servers/server-vulnerabilities.svelte';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as DropdownMenu from '#lib/components/ui/dropdown-menu/index.js';
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import { getMe } from '#lib/clients/auth-client.js';
	import { getServerMetricsHistory } from '#lib/clients/container-metrics-client.js';
	import { listMembers } from '#lib/clients/organization-client.js';
	import { getRuntimeStatus, type RuntimeStatus } from '#lib/clients/runtime-client.js';
	import {
		getServer,
		healthcheckServer,
		listServerApplications,
		listServerPorts,
		setupServer,
		type Server,
		type ServerAssignedApplication,
		type ServerPort,
		type ServerState,
	} from '#lib/clients/server-client.js';
	import { environmentApplicationsHref } from '#lib/utils/environment-route.js';
	import { osLabel } from '#lib/utils/os-label.js';
	import { canOpenLocalConsole } from '#lib/utils/server-visibility.js';
	import { nameCurrentCrumb } from '#lib/state/page-header.svelte.js';
	import { setSelectedOrganizationId } from '#lib/utils/selected-organization.js';
	import { toast } from '#lib/utils/toast.js';
	import { t } from '#lib/i18n/index.svelte.js';

	const organizationId = $derived(page.params.id!);
	const serverId = $derived(page.params.server_id!);

	let server = $state<Server | null>(null);
	let loadError = $state('');
	let runtime = $state<RuntimeStatus | null>(null);
	let applications = $state<ServerAssignedApplication[] | null>(null);
	let ports = $state<ServerPort[]>([]);
	let isSysadmin = $state(false);
	let isAdmin = $state(false);
	let busy = $state<'health' | 'repair' | null>(null);

	let tab = $state('overview');
	/** The embedded shell stays connected once opened, so switching tabs does not drop it. */
	let terminalOpened = $state(false);
	let fullscreenOpen = $state(false);
	let maintenanceOpen = $state(false);
	let securityOpen = $state(false);
	let renameOpen = $state(false);

	const isRemote = $derived(server?.kind === 'ssh');
	// Same rules as the servers list: the local host is the SysAdmin's alone.
	const canManage = $derived(isRemote ? isAdmin || isSysadmin : isSysadmin);
	const canConsole = $derived(
		!!server &&
			(isRemote
				? canManage && server.state === 'ready'
				: canOpenLocalConsole({ isSysadmin, kind: server.kind, state: server.state })),
	);
	const live = $derived(server?.state === 'ready' || server?.state === 'degraded');

	const STATE_CLASSES: Record<ServerState, string> = {
		ready: 'border-success/40 bg-success/10 text-success',
		setting_up: 'border-info/40 bg-info/10 text-info',
		needs_verification: 'border-warning/40 bg-warning/10 text-warning',
		needs_setup: 'border-warning/40 bg-warning/10 text-warning',
		degraded: 'border-destructive/40 bg-destructive/10 text-destructive',
		offline: 'border-destructive/40 bg-destructive/10 text-destructive',
		unsupported: 'border-destructive/40 bg-destructive/10 text-destructive',
	};

	const address = $derived(
		!server
			? ''
			: isRemote
				? `${server.ssh_user}@${server.ssh_host}:${server.ssh_port}`
				: (server.public_address ?? t('organizations.servers.installed_locally')),
	);
	const os = $derived(
		!server
			? null
			: isRemote
				? osLabel(server.facts.os, server.facts.version)
				: osLabel(runtime?.host_distribution ?? runtime?.host_os),
	);

	function gib(bytes: number | null | undefined): string | null {
		return bytes ? `${(bytes / 1024 ** 3).toFixed(1)} GiB` : null;
	}

	const specs = $derived(
		!server
			? []
			: (
					[
						['os', [os, !isRemote ? runtime?.host_version : null].filter(Boolean).join(' ') || null],
						[
							'cpu',
							server.facts.capacity.cpu_cores
								? t('servers.card.vcpu', { count: server.facts.capacity.cpu_cores })
								: null,
						],
						['memory', gib(server.facts.capacity.memory_bytes)],
						[
							'disk',
							gib(server.facts.capacity.disk_free_bytes)
								? t('servers.card.free', { value: gib(server.facts.capacity.disk_free_bytes) })
								: null,
						],
						['podman', server.facts.podman_version],
						['arch', server.facts.architecture],
						['kernel', !isRemote ? (runtime?.host_kernel_version ?? null) : null],
					] as [string, string | null | undefined][]
				).filter((spec): spec is [string, string] => Boolean(spec[1])),
	);

	const tabs = $derived([
		{ value: 'overview', label: t('servers.detail.tab_overview') },
		...(live ? [{ value: 'metrics', label: t('servers.card.history') }] : []),
		...(canConsole ? [{ value: 'terminal', label: t('servers.detail.tab_terminal') }] : []),
		...(isRemote && canManage ? [{ value: 'security', label: t('servers.security.menu') }] : []),
	]);
	const visibleTab = $derived(tabs.some((item) => item.value === tab) ? tab : 'overview');

	function openTab(value: string) {
		tab = value;
		if (value === 'terminal') terminalOpened = true;
	}

	async function load() {
		try {
			server = await getServer(organizationId, serverId);
			loadError = '';
			nameCurrentCrumb(server.name, page.url.pathname);
		} catch (caught) {
			server = null;
			loadError = caught instanceof Error ? caught.message : t('organizations.servers.load_failed');
		}
	}

	$effect(() => {
		setSelectedOrganizationId(organizationId);
		void load();
		// Side panels only enrich the page, so a failure leaves them empty.
		listServerApplications(organizationId, serverId).then(
			(result) => (applications = result),
			() => (applications = []),
		);
		listServerPorts(organizationId, serverId).then(
			(result) => (ports = result),
			() => (ports = []),
		);
		getMe()
			.then(async (me) => {
				isSysadmin = me.is_sysadmin;
				isAdmin = (await listMembers(organizationId)).some(
					(member) => member.user_id === me.user_id && member.role_id <= 1,
				);
			})
			.catch(() => {});
		getRuntimeStatus().then(
			(result) => (runtime = result),
			() => (runtime = null),
		);
	});

	async function run(action: 'health' | 'repair') {
		if (!server) return;
		busy = action;
		try {
			if (action === 'health') {
				await healthcheckServer(organizationId, server.id);
				toast.success(t('organizations.servers.healthy_toast', { name: server.name }));
			} else {
				await setupServer(organizationId, server.id, true);
				toast.success(t('organizations.servers.repair_queued_toast', { name: server.name }));
			}
			await load();
		} catch (caught) {
			toast.error(
				caught instanceof Error ? caught.message : t('organizations.servers.action_failed'),
			);
		} finally {
			busy = null;
		}
	}

	type QuickAction = {
		icon: typeof TerminalIcon;
		label: string;
		description: string;
		run?: () => void;
		href?: string;
	};
	const quickActions = $derived(
		(
			[
			canConsole && {
				icon: TerminalIcon,
				label: t('servers.detail.tab_terminal'),
				description: t('servers.detail.terminal_desc'),
				run: () => openTab('terminal'),
			},
			canManage && {
				icon: WrenchIcon,
				label: t('organizations.servers.maintenance'),
				description: t('servers.detail.maintenance_desc'),
				run: () => (maintenanceOpen = true),
			},
			isRemote &&
				canManage && {
					icon: ShieldCheckIcon,
					label: t('servers.security.menu'),
					description: t('servers.detail.security_desc'),
					run: () => (securityOpen = true),
				},
			isRemote &&
				canManage && {
					icon: ActivityIcon,
					label: t('organizations.servers.health_check'),
					description: t('servers.detail.health_desc'),
					run: () => void run('health'),
				},
			// The containers browser lives on the SysAdmin-only System page.
			isSysadmin &&
				live && {
					icon: ContainerIcon,
					label: t('menu.navigation.containers'),
					description: t('servers.detail.containers_desc'),
					href: `/system?tab=containers&server=${encodeURIComponent(serverId)}`,
				},
			] as (QuickAction | false)[]
		).filter((action): action is QuickAction => Boolean(action)),
	);
</script>

<div class="flex w-full flex-col gap-6 px-4 py-6 sm:px-6">
	<div class="flex flex-wrap items-start gap-3">
		<Button
			variant="ghost"
			size="icon-sm"
			href={`/organizations/${encodeURIComponent(organizationId)}/servers`}
			aria-label={t('organizations.servers.title')}>
			<ArrowLeftIcon class="size-4" />
		</Button>
		<div class="min-w-48 flex-1">
			{#if server}
				<div class="flex flex-wrap items-center gap-2">
					{#if os}
						<span class="flex shrink-0 items-center" title={os}>
							<OsIcon
								os={isRemote ? null : runtime?.host_os}
								distribution={isRemote ? server.facts.os : runtime?.host_distribution}
								class="size-5" />
						</span>
					{/if}
					<h1 class="truncate text-xl font-semibold tracking-tight">{server.name}</h1>
					<Badge variant="outline" class={STATE_CLASSES[server.state]}
						>{t(`organizations.servers.${server.state}`)}</Badge>
					{#if server.vulnerabilities?.urgent}
						<Badge variant="destructive">
							{t('servers.security.vuln_card_badge', { count: server.vulnerabilities.urgent })}
						</Badge>
					{/if}
				</div>
				<CopyButton
					value={address}
					label={address}
					class="mt-1 max-w-full font-mono text-[12.5px] text-muted-foreground hover:text-foreground" />
			{:else if !loadError}
				<Skeleton class="h-7 w-56" />
				<Skeleton class="mt-2 h-4 w-40" />
			{/if}
		</div>
		{#if server}
			<div class="flex shrink-0 flex-wrap items-center gap-1.5">
				{#if canConsole}
					<Button size="sm" onclick={() => openTab('terminal')}>
						<TerminalIcon class="size-4" />
						{t('organizations.servers.open_console')}
					</Button>
				{/if}
				{#if canManage}
					<Button size="sm" variant="outline" onclick={() => (maintenanceOpen = true)}>
						<WrenchIcon class="size-4" />
						{t('organizations.servers.maintenance')}
					</Button>
					<DropdownMenu.Root>
						<DropdownMenu.Trigger>
							{#snippet child({ props })}
								<Button
									{...props}
									size="icon-sm"
									variant="outline"
									disabled={busy !== null}
									aria-label={t('organizations.servers.actions')}>
									<EllipsisVerticalIcon class="size-4" />
								</Button>
							{/snippet}
						</DropdownMenu.Trigger>
						<DropdownMenu.Content align="end">
							{#if isRemote}
								<DropdownMenu.Item onSelect={() => void run('health')}>
									<ActivityIcon class="size-4" />
									{t('organizations.servers.health_check')}
								</DropdownMenu.Item>
								<DropdownMenu.Item onSelect={() => void run('repair')}>
									<WrenchIcon class="size-4" />
									{t('organizations.servers.repair')}
								</DropdownMenu.Item>
								<DropdownMenu.Item onSelect={() => (securityOpen = true)}>
									<ShieldCheckIcon class="size-4" />
									{t('servers.security.menu')}
								</DropdownMenu.Item>
							{/if}
							<DropdownMenu.Item onSelect={() => (renameOpen = true)}>
								<PencilIcon class="size-4" />
								{t('organizations.servers.rename_action')}
							</DropdownMenu.Item>
						</DropdownMenu.Content>
					</DropdownMenu.Root>
				{/if}
			</div>
		{/if}
	</div>

	{#if loadError}
		<p
			class="rounded-lg border border-destructive/30 bg-destructive/5 p-4 text-sm text-destructive">
			{loadError}
		</p>
	{:else if server}
		{#if tabs.length > 1}
			<AnimatedTabs items={tabs} value={visibleTab} onValueChange={openTab} shortcuts={false} />
		{/if}

		{#if visibleTab === 'overview'}
			{#if server.last_error}
				<p
					class="rounded-lg border border-destructive/30 bg-destructive/5 p-3 text-sm text-destructive">
					{server.last_error.message}
				</p>
			{/if}
			{#if isRemote && canManage && server.facts.helper?.outdated}
				<ServerHelperNotice {organizationId} {server} onUpdated={() => void run('health')} />
			{/if}
			{#if isRemote && canManage && server.state === 'ready' && server.facts.swap_recommended}
				<ServerSwapNotice {organizationId} {server} onCreated={() => void run('health')} />
			{/if}

			{#if quickActions.length}
				<div class="grid grid-cols-[repeat(auto-fill,minmax(min(100%,220px),1fr))] gap-3">
					{#each quickActions as action (action.label)}
						{@const Icon = action.icon}
						{#snippet body()}
							<span
								class="grid size-9 shrink-0 place-items-center rounded-xl border bg-muted/60 text-foreground transition-colors group-hover:border-primary/40 group-hover:text-primary">
								<Icon class="size-4" />
							</span>
							<span class="min-w-0">
								<span class="flex items-center gap-1 text-sm font-medium">
									{action.label}
									{#if action.href}<ExternalLinkIcon class="size-3 text-muted-foreground" />{/if}
								</span>
								<span class="mt-0.5 block text-xs text-muted-foreground">{action.description}</span>
							</span>
						{/snippet}
						{#if action.href}
							<a
								href={action.href}
								class="group lp-raised flex items-start gap-3 rounded-2xl border bg-card p-4 text-left transition-colors hover:bg-accent/40">
								{@render body()}
							</a>
						{:else}
							<button
								type="button"
								disabled={busy !== null}
								onclick={action.run}
								class="group lp-raised flex items-start gap-3 rounded-2xl border bg-card p-4 text-left transition-colors hover:bg-accent/40 disabled:opacity-60">
								{@render body()}
							</button>
						{/if}
					{/each}
				</div>
			{/if}

			<div class="grid gap-4 lg:grid-cols-2">
				<section class="lp-raised flex flex-col gap-3 rounded-2xl border bg-card p-4 sm:p-5">
					<h2 class="text-sm font-semibold">{t('servers.detail.system')}</h2>
					{#if specs.length}
						<dl class="grid grid-cols-2 gap-2.5 sm:grid-cols-3">
							{#each specs as [key, value] (key)}
								<div
									class="flex min-w-0 flex-col gap-0.5 rounded-lg bg-muted/60 px-3 py-2.5 lp-inset">
									<dt class="font-mono text-[11.5px] text-muted-foreground">
										{t(`servers.card.spec_${key}`)}
									</dt>
									<dd class="truncate text-[13px] font-medium" title={value}>{value}</dd>
								</div>
							{/each}
						</dl>
					{/if}
					<div class="flex flex-wrap gap-x-4 gap-y-1 text-xs text-muted-foreground">
						<span>{t('servers.detail.ingress')}: {server.ingress_mode.replaceAll('_', ' ')}</span>
						{#if isRemote}<span>Podman: {server.podman_mode}</span>{/if}
					</div>
					{#if server.host_key_fingerprint}
						<CopyButton
							value={server.host_key_fingerprint}
							label={server.host_key_fingerprint}
							copiedLabel={t('servers.card.fingerprint_copied')}
							class="max-w-full font-mono text-xs text-muted-foreground hover:text-foreground" />
					{/if}
				</section>

				<section class="lp-raised flex flex-col gap-3 rounded-2xl border bg-card p-4 sm:p-5">
					<h2 class="flex items-center gap-2 text-sm font-semibold">
						{t('servers.detail.applications')}
						<Badge variant="secondary">{applications?.length ?? server.assigned_application_count}</Badge>
					</h2>
					{#if applications === null}
						<Skeleton class="h-16 w-full" />
					{:else if applications.length === 0}
						<p class="text-sm text-muted-foreground">{t('servers.detail.no_applications')}</p>
					{:else}
						<ul class="-mx-1 max-h-72 space-y-0.5 overflow-auto text-sm">
							{#each applications as application (application.id)}
								<li>
									<a
										class="flex items-center gap-2 rounded-lg px-2 py-1.5 hover:bg-accent/50"
										href={`${environmentApplicationsHref(application.project_id, application.environment_name)}/${encodeURIComponent(application.id)}`}>
										<span class="truncate font-medium">{application.name}</span>
										<span class="truncate text-xs text-muted-foreground">
											{application.project_name} / {application.environment_name}
										</span>
									</a>
								</li>
							{/each}
						</ul>
					{/if}
				</section>
			</div>

			{#if ports.length}
				<section class="lp-raised flex flex-col gap-3 rounded-2xl border bg-card p-4 sm:p-5">
					<h2 class="text-sm font-semibold">{t('servers.detail.ports')}</h2>
					<div class="overflow-x-auto">
						<table class="w-full text-left text-sm">
							<tbody>
								{#each ports as port, index (index)}
									<tr class="border-t first:border-t-0">
										<td class="py-1.5 pr-4 font-mono text-xs"
											>{port.host_ip ? `${port.host_ip}:` : ''}{port.host_port} → {port.container_port}/{port.protocol}</td>
										<td class="py-1.5 pr-4 truncate">{port.application_name}</td>
										<td class="py-1.5 text-xs text-muted-foreground">{port.service ?? ''}</td>
									</tr>
								{/each}
							</tbody>
						</table>
					</div>
				</section>
			{/if}
		{:else if visibleTab === 'metrics'}
			<div class="lp-raised rounded-2xl border bg-card p-4 sm:p-5">
				{#key serverId}
					<MetricsHistoryExplorer
						host
						memoryFallback={server.facts?.capacity?.memory_bytes ?? null}
						load={(window) => getServerMetricsHistory(organizationId, serverId, window)} />
				{/key}
			</div>
		{:else if visibleTab === 'security'}
			<div class="lp-raised flex flex-col gap-4 rounded-2xl border bg-card p-4 sm:p-5">
				<div class="flex flex-wrap items-center justify-between gap-2">
					<p class="max-w-prose text-sm text-muted-foreground">{t('servers.detail.security_desc')}</p>
					<Button size="sm" variant="outline" onclick={() => (securityOpen = true)}>
						<ShieldCheckIcon class="size-4" />
						{t('servers.detail.hardening')}
					</Button>
				</div>
				<ServerVulnerabilities {organizationId} {serverId} />
			</div>
		{/if}

		{#if terminalOpened && canConsole}
			<div
				class:hidden={visibleTab !== 'terminal'}
				class="lp-raised flex flex-col overflow-hidden rounded-2xl border bg-card">
				<div class="flex items-center justify-end border-b bg-muted/20 px-3 py-1.5">
					<Button size="sm" variant="ghost" onclick={() => (fullscreenOpen = true)}>
						<Maximize2Icon class="size-4" />
						{t('servers.detail.fullscreen')}
					</Button>
				</div>
				<div class="relative h-[70dvh] min-h-80">
					<ServerTerminalTab active={visibleTab === 'terminal'} {organizationId} {serverId} />
				</div>
			</div>
		{/if}
	{/if}
</div>

<ServerMaintenanceDialog
	bind:open={maintenanceOpen}
	{organizationId}
	{server}
	onDone={() => void load()} />
<ServerSecurityDialog bind:open={securityOpen} {organizationId} {server} />
<RenameServerDialog bind:open={renameOpen} {organizationId} {server} onRenamed={() => void load()} />
<ServerTerminalDialog
	open={fullscreenOpen}
	{organizationId}
	server={fullscreenOpen ? server : null}
	onOpenChange={(next) => (fullscreenOpen = next)} />
