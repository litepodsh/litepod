<script lang="ts">
	import { onMount } from 'svelte';
	import { page } from '$app/state';
	import ChevronRightIcon from '@lucide/svelte/icons/chevron-right';
	import ExternalLinkIcon from '@lucide/svelte/icons/external-link';
	import HouseIcon from '@lucide/svelte/icons/house';
	import PlusIcon from '@lucide/svelte/icons/plus';
	import ServerIcon from '@lucide/svelte/icons/server';
	import * as AlertDialog from '#lib/components/ui/alert-dialog/index.js';
	import DeleteConfirmControl from '#lib/components/custom/delete-confirm-control.svelte';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Collapsible from '#lib/components/ui/collapsible/index.js';
	import * as Empty from '#lib/components/ui/empty/index.js';
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import ConnectServerDialog from '#lib/components/servers/connect-server-dialog.svelte';
	import RenameServerDialog from '#lib/components/servers/rename-server-dialog.svelte';
	import ServerCard from '#lib/components/servers/server-card.svelte';
	import ServerGlobe from '#lib/components/servers/server-globe.svelte';
	import ServerMaintenanceDialog from '#lib/components/servers/server-maintenance-dialog.svelte';
	import ServerTerminalDialog from '#lib/components/servers/server-terminal-dialog.svelte';
	import { getMe } from '#lib/clients/auth-client.js';
	import { listMembers } from '#lib/clients/organization-client.js';
	import { getRuntimeStatus, type RuntimeStatus } from '#lib/clients/runtime-client.js';
	import { getSystemVersion, type SystemVersion } from '#lib/clients/system-client.js';
	import * as Dialog from '#lib/components/ui/dialog/index.js';
	import {
		deleteServer,
		getRemoteServerCapacity,
		healthcheckServer,
		listServerApplications,
		listServerGeo,
		listServers,
		setupServer,
		testDeployServer,
		type Server,
		type ServerAssignedApplication,
		type ServerGeo,
		type RemoteServerCapacity,
		type ServerTestDeploy,
	} from '#lib/clients/server-client.js';
	import ServerGroupDialog from '#lib/components/server-groups/server-group-dialog.svelte';
	import ServerGroupMark from '#lib/components/server-groups/server-group-mark.svelte';
	import BoxesIcon from '@lucide/svelte/icons/boxes';
	import WaypointsIcon from '@lucide/svelte/icons/waypoints';
	import type { ServerSummaryDto } from '#lib/clients/project-client.js';
	import {
		listServerGroups,
		replaceServerGroupServers,
		type ServerGroup,
	} from '#lib/clients/server-group-client.js';
	import { environmentApplicationsHref } from '#lib/utils/environment-route.js';
	import { canOpenLocalConsole, serverGroups } from '#lib/utils/server-visibility.js';
	import { t } from '#lib/i18n/index.svelte.js';
	import { toast } from '#lib/utils/toast.js';

	const organizationId = page.params.id!;

	let servers = $state<Server[]>([]);
	let systemVersion = $state<SystemVersion | null>(null);
	/** Admins (role 1) and the SysAdmin manage servers; everyone else reads. */
	let canManage = $state(false);
	let isSysadmin = $state(false);
	let remoteCapacity = $state<RemoteServerCapacity | null>(null);
	/** The local server is never probed over SSH, so its OS comes from the API host itself. */
	let runtime = $state<RuntimeStatus | null>(null);
	let loading = $state(true);
	let error = $state('');
	/** Keyed `${serverId}:${action}` so one row's health check does not freeze the whole card. */
	let busy = $state<string | null>(null);

	let wizardOpen = $state(false);
	let wizardResume = $state<Server | null>(null);
	let wizardStep = $state<number | null>(null);

	let pendingDelete = $state<Server | null>(null);
	let maintenanceServer = $state<Server | null>(null);
	let maintenanceOpen = $state(false);
	let renameTarget = $state<Server | null>(null);
	let renameOpen = $state(false);
	/** Applications still on the server being removed; removal stays blocked while this is non-empty. */
	let pendingDeleteApplications = $state<ServerAssignedApplication[] | null>(null);
	let consoleServer = $state<Server | null>(null);

	function askDelete(server: Server) {
		pendingDelete = server;
		pendingDeleteApplications = null;
		void listServerApplications(organizationId, server.id)
			.then((applications) => {
				// A slower response must not overwrite a dialog the user already moved on from.
				if (pendingDelete?.id === server.id) pendingDeleteApplications = applications;
			})
			.catch(() => {
				if (pendingDelete?.id === server.id) pendingDeleteApplications = [];
			});
	}

	/** Container output is multi-line, so it gets a dialog instead of a toast. */
	let testResult = $state<{ server: Server; result: ServerTestDeploy } | null>(null);

	const blockingApplications = $derived(pendingDeleteApplications ?? []);
	const isCloud = $derived(systemVersion?.is_cloud ?? false);
	// On cloud the API returns the control plane's `local` server to a SysAdmin only.
	const groupedServers = $derived(serverGroups(servers, isCloud && !isSysadmin));
	const remoteServers = $derived(groupedServers.remote);
	const localServers = $derived(groupedServers.local);
	const settingUp = $derived(servers.some((server) => server.state === 'setting_up'));
	const canConnectRemote = $derived(
		canManage && (!isCloud || isSysadmin || remoteCapacity?.available === true),
	);
	const remoteCapacityReason = $derived(isCloud && !isSysadmin ? remoteCapacity?.reason : null);

	let geo = $state<ServerGeo[]>([]);
	let geoLoading = $state(true);
	let geoError = $state<string | null>(null);

	/** Geolocation calls out to ip.guide, so a failure only costs the globe, never the page. */
	async function loadGeo() {
		geoLoading = true;
		geoError = null;
		try {
			geo = await listServerGeo(organizationId);
		} catch (caught) {
			geoError =
				caught instanceof Error ? caught.message : t('organizations.topology.globe_load_failed');
		} finally {
			geoLoading = false;
		}
	}

	let remoteOpen = $state(true);
	let localOpen = $state(true);

	let groups = $state<ServerGroup[]>([]);
	/** Remote cards ticked for "Group selected". */
	let selection = $state<string[]>([]);
	let groupDialogOpen = $state(false);
	let groupPreselected = $state<string[]>([]);

	/** Every remote server can join a group, so the dialog lists them all, not only ready ones. */
	const groupCandidates = $derived(
		remoteServers.map(
			(server): ServerSummaryDto => ({
				id: server.id,
				kind: server.kind,
				name: server.name,
				state: server.state,
				ingress_mode: server.ingress_mode,
				capacity: server.facts.capacity,
				os: null,
				distribution: server.facts.os,
				os_version: server.facts.version,
			}),
		),
	);

	/** One section per non-empty group, then the rest; a server in two groups shows in both. */
	const remoteSections = $derived.by(() => {
		const sections = groups
			.map((group) => ({
				group: group as ServerGroup | null,
				servers: remoteServers.filter((server) =>
					group.member_servers.some((member) => member.id === server.id),
				),
			}))
			.filter((section) => section.servers.length > 0);
		const grouped = new Set(sections.flatMap((section) => section.servers.map((s) => s.id)));
		const rest = remoteServers.filter((server) => !grouped.has(server.id));
		if (rest.length || !sections.length) sections.push({ group: null, servers: rest });
		return sections;
	});

	/** Groups only cost the sections, so a failed read leaves the plain list. */
	async function loadGroups() {
		groups = await listServerGroups(organizationId).catch(() => groups);
	}

	function openGroupDialog(serverIds: string[]) {
		groupPreselected = serverIds;
		groupDialogOpen = true;
	}

	async function toggleGroup(server: Server, group: ServerGroup, member: boolean) {
		const others = group.member_servers.map((entry) => entry.id).filter((id) => id !== server.id);
		try {
			await replaceServerGroupServers(
				organizationId,
				group.id,
				member ? [...others, server.id] : others,
			);
			toast.success(
				t(
					member
						? 'organizations.server_groups.added_toast'
						: 'organizations.server_groups.removed_toast',
					{ server: server.name, group: group.name },
				),
			);
		} catch (caught) {
			toast.error(
				caught instanceof Error ? caught.message : t('organizations.server_groups.save_failed'),
			);
		}
		await loadGroups();
	}

	async function refresh() {
		servers = await listServers(organizationId);
	}

	onMount(() => {
		let timer: ReturnType<typeof setInterval> | undefined;
		void loadGeo();
		void (async () => {
			try {
				const me = await getMe();
				isSysadmin = me.is_sysadmin;
				// A SysAdmin manages every organization's servers without being a
				// member; otherwise it takes the Admin role inside this one.
				canManage =
					me.is_sysadmin ||
					(await listMembers(organizationId)).some(
						(member) => member.user_id === me.user_id && member.role_id <= 1,
					);
				systemVersion = await getSystemVersion();
				await Promise.all([refresh(), loadGroups()]);
				if (systemVersion.is_cloud && canManage && !isSysadmin) {
					remoteCapacity = await getRemoteServerCapacity(organizationId).catch((caught) => ({
						available: false,
						reason:
							caught instanceof Error ? caught.message : t('organizations.servers.capacity_failed'),
					}));
				}
				// A missing runtime only costs the local card its OS mark, so it never fails the page.
				if (!systemVersion.is_cloud) runtime = await getRuntimeStatus().catch(() => null);
				// Setup runs server-side; poll so the card reaches Ready without a manual reload.
				timer = setInterval(() => {
					if (settingUp && !wizardOpen) void refresh().catch(() => {});
				}, 5000);
			} catch (caught) {
				error = caught instanceof Error ? caught.message : t('organizations.servers.load_failed');
			} finally {
				loading = false;
			}
		})();
		return () => clearInterval(timer);
	});

	function connect() {
		if (!canConnectRemote) return;
		wizardResume = null;
		wizardStep = null;
		wizardOpen = true;
	}

	function resume(server: Server, step: number | null = null) {
		wizardResume = server;
		wizardStep = step;
		wizardOpen = true;
	}

	function openMaintenance(server: Server) {
		maintenanceServer = server;
		maintenanceOpen = true;
	}

	function openRename(server: Server) {
		renameTarget = server;
		renameOpen = true;
	}

	async function run(server: Server, action: 'health' | 'repair') {
		busy = `${server.id}:${action}`;
		try {
			if (action === 'health') {
				await healthcheckServer(organizationId, server.id);
				toast.success(t('organizations.servers.healthy_toast', { name: server.name }));
			} else {
				await setupServer(organizationId, server.id, true);
				toast.success(t('organizations.servers.repair_queued_toast', { name: server.name }));
			}
			await refresh();
		} catch (caught) {
			toast.error(
				caught instanceof Error ? caught.message : t('organizations.servers.action_failed'),
			);
		} finally {
			busy = null;
		}
	}

	/** A failed container is a normal answer here, so only a transport error becomes a toast. */
	async function testDeploy(server: Server) {
		busy = `${server.id}:test`;
		try {
			testResult = { server, result: await testDeployServer(organizationId, server.id) };
		} catch (caught) {
			toast.error(
				caught instanceof Error ? caught.message : t('organizations.servers.test_failed'),
			);
		} finally {
			busy = null;
		}
	}

	async function confirmDelete() {
		const target = pendingDelete;
		if (!target) return;
		busy = `${target.id}:delete`;
		try {
			await deleteServer(organizationId, target.id);
			pendingDelete = null;
			selection = selection.filter((id) => id !== target.id);
			await Promise.all([refresh(), loadGroups()]);
			toast.success(t('organizations.servers.removed_toast', { name: target.name }));
		} catch (caught) {
			toast.error(
				caught instanceof Error ? caught.message : t('organizations.servers.remove_failed'),
			);
		} finally {
			busy = null;
		}
	}
</script>

<!-- No width cap of its own: `main.shell-col` sets it, so the header's full-width toggle widens this page too. -->
<div class="flex w-full flex-col gap-6 px-4 py-6 sm:px-6">
	<div class="flex flex-col gap-4 sm:flex-row sm:items-start sm:justify-between">
		<div>
			<h1 class="text-xl font-semibold tracking-tight">{t('organizations.servers.title')}</h1>
			<p class="mt-1 text-sm text-muted-foreground">
				{isCloud
					? t('organizations.servers.cloud_description')
					: t('organizations.servers.description')}
			</p>
			<div class="mt-2 flex flex-wrap gap-2">
				<Button
					variant="outline"
					size="sm"
					href={`/organizations/${encodeURIComponent(organizationId)}/server-groups`}>
					<BoxesIcon class="size-4" />
					{t('menu.navigation.server_groups')}
				</Button>
				<Button
					variant="outline"
					size="sm"
					href={`/organizations/${encodeURIComponent(organizationId)}/topology`}>
					<WaypointsIcon class="size-4" />
					{t('menu.navigation.map')}
				</Button>
			</div>
		</div>
		{#if !error && canManage}
			<div class="flex flex-col items-start gap-1 sm:items-end">
				<div class="flex flex-wrap items-center gap-2">
					{#if remoteServers.length}
						<Button variant="outline" onclick={() => openGroupDialog(selection)}>
							<BoxesIcon class="size-4" />
							{selection.length
								? t('organizations.server_groups.group_selected', { count: selection.length })
								: t('organizations.server_groups.new_group')}
						</Button>
					{/if}
					<Button onclick={connect} disabled={!canConnectRemote}>
						<PlusIcon class="size-4" />
						{t('organizations.servers.connect')}
					</Button>
				</div>
				{#if remoteCapacityReason}
					<p class="text-xs text-muted-foreground sm:text-right">{remoteCapacityReason}</p>
				{/if}
			</div>
		{/if}
	</div>

	<ServerGlobe servers={geo} loading={geoLoading} error={geoError} {isCloud} onRefresh={loadGeo} />

	{#if loading}
		<div class="grid gap-4">
			{#each [0, 1] as placeholder (placeholder)}
				<Skeleton class="h-32 w-full rounded-xl" />
			{/each}
		</div>
	{:else if error}
		<p
			class="rounded-lg border border-destructive/30 bg-destructive/5 p-4 text-sm text-destructive">
			{error}
		</p>
	{:else}
		{#if localServers.length > 0}
			<Collapsible.Root bind:open={localOpen} class="space-y-3">
				<Collapsible.Trigger class="group flex w-full items-center gap-2 text-left">
					<ChevronRightIcon
						class="size-4 shrink-0 text-muted-foreground transition-transform group-data-[state=open]:rotate-90" />
					<HouseIcon class="size-4 shrink-0 text-muted-foreground" />
					<h2 class="text-sm font-semibold">
						{t(
							isCloud ? 'organizations.servers.cloud_server' : 'organizations.servers.this_machine',
						)}
					</h2>
					<Badge variant="secondary">{localServers.length}</Badge>
					<span class="ml-auto hidden text-xs text-muted-foreground sm:inline"
						>{t(
							isCloud
								? 'organizations.servers.cloud_server_hint'
								: 'organizations.servers.this_machine_hint',
						)}</span>
				</Collapsible.Trigger>
				<Collapsible.Content
					class="lp-shadow-room grid gap-4 overflow-hidden data-[state=closed]:animate-collapsible-up data-[state=open]:animate-collapsible-down">
					{#each localServers as server, index (server.id)}
						<div
							class="motion-safe:animate-in motion-safe:fade-in motion-safe:slide-in-from-top-2 motion-safe:fill-mode-both motion-safe:duration-300"
							style="animation-delay: {index * 60}ms">
							<ServerCard
								{server}
								{runtime}
								canManage={canManage && isSysadmin}
								busy={busy?.startsWith(`${server.id}:`) ?? false}
								onContinue={(target) => resume(target)}
								onShowKey={(target) => resume(target, 2)}
								onHealthcheck={(target) => void run(target, 'health')}
								onMaintenance={openMaintenance}
								onTestDeploy={(target) => void testDeploy(target)}
								onRepair={(target) => void run(target, 'repair')}
								onRename={openRename}
								onDelete={(target) => askDelete(target)}
								onOpenConsole={canOpenLocalConsole({
									isSysadmin,
									kind: server.kind,
									state: server.state,
								})
									? (target) => (consoleServer = target)
									: undefined} />
						</div>
					{/each}
				</Collapsible.Content>
			</Collapsible.Root>
		{/if}

		<Collapsible.Root bind:open={remoteOpen} class="space-y-3">
			<Collapsible.Trigger class="group flex w-full items-center gap-2 text-left">
				<ChevronRightIcon
					class="size-4 shrink-0 text-muted-foreground transition-transform group-data-[state=open]:rotate-90" />
				<ServerIcon class="size-4 shrink-0 text-muted-foreground" />
				<h2 class="text-sm font-semibold">{t('organizations.servers.remote')}</h2>
				<Badge variant="secondary">{remoteServers.length}</Badge>
				<span class="ml-auto hidden text-xs text-muted-foreground sm:inline"
					>{t('organizations.servers.remote_hint')}</span>
			</Collapsible.Trigger>
			<Collapsible.Content
				class="lp-shadow-room overflow-hidden data-[state=closed]:animate-collapsible-up data-[state=open]:animate-collapsible-down">
				{#if remoteServers.length === 0}
					<Empty.Root class="rounded-xl border border-dashed">
						<Empty.Header>
							<Empty.Media variant="icon">
								<ServerIcon />
							</Empty.Media>
							<Empty.Title>{t('organizations.servers.empty')}</Empty.Title>
							<Empty.Description>
								Connect an Ubuntu or Debian host and litepod will install Podman, pin its SSH host
								key, and make it available as a deployment target.
							</Empty.Description>
						</Empty.Header>
						<Empty.Content>
							<Button onclick={connect} disabled={!canConnectRemote}>
								<PlusIcon class="size-4" />
								Connect a remote server
							</Button>
						</Empty.Content>
					</Empty.Root>
				{:else}
					{#each remoteSections as section, sectionIndex (section.group?.id ?? '')}
						{#if remoteSections.length > 1}
							<div
								class="mb-3 flex items-center gap-2 text-sm font-medium {sectionIndex
									? 'mt-6'
									: ''}">
								{#if section.group}
									<ServerGroupMark group={section.group} class="text-sm text-foreground" />
								{:else}
									<span class="text-muted-foreground"
										>{t('organizations.server_groups.ungrouped')}</span>
								{/if}
								<Badge variant="secondary">{section.servers.length}</Badge>
							</div>
						{/if}
						<div class="grid grid-cols-[repeat(auto-fill,minmax(min(100%,320px),1fr))] gap-4">
							{#each section.servers as server, index (server.id)}
								<div
									class="motion-safe:animate-in motion-safe:fade-in motion-safe:slide-in-from-top-2 motion-safe:fill-mode-both motion-safe:duration-300"
									style="animation-delay: {index * 60}ms">
									<ServerCard
										{server}
										{canManage}
										busy={busy?.startsWith(`${server.id}:`) ?? false}
										onContinue={(target) => resume(target)}
										onShowKey={(target) => resume(target, 2)}
										onHealthcheck={(target) => void run(target, 'health')}
										onMaintenance={openMaintenance}
										onTestDeploy={(target) => void testDeploy(target)}
										onRepair={(target) => void run(target, 'repair')}
										onRename={openRename}
										onDelete={(target) => askDelete(target)}
										onOpenConsole={canManage ? (target) => (consoleServer = target) : undefined}
										groups={canManage ? groups : undefined}
										onToggleGroup={(target, group, member) =>
											void toggleGroup(target, group, member)}
										onNewGroup={(target) => openGroupDialog([target.id])}
										selected={selection.includes(server.id)}
										onSelectedChange={canManage
											? (target, next) =>
													(selection = next
														? [...selection, target.id]
														: selection.filter((id) => id !== target.id))
											: undefined} />
								</div>
							{/each}
							{#if canManage && sectionIndex === remoteSections.length - 1}
								<div
									class="flex flex-col justify-between gap-4 rounded-[15px] border border-dashed border-foreground/20 p-4.5">
									<div>
										<h3 class="text-[15px] font-semibold">
											{t('organizations.servers.connect_another')}
										</h3>
										<p class="mt-1.5 max-w-[44ch] text-[13px] text-muted-foreground">
											{t('organizations.servers.connect_another_hint')}
										</p>
									</div>
									<Button
										variant="outline"
										class="self-start"
										onclick={connect}
										disabled={!canConnectRemote}>
										<PlusIcon class="size-4" />
										{t('organizations.servers.connect')}
									</Button>
								</div>
							{/if}
						</div>
					{/each}
				{/if}
			</Collapsible.Content>
		</Collapsible.Root>
	{/if}
</div>

<ConnectServerDialog
	bind:open={wizardOpen}
	{organizationId}
	cloudMode={isCloud}
	{isSysadmin}
	resume={wizardResume}
	resumeStep={wizardStep}
	onDone={() => void refresh()} />

<ServerGroupDialog
	bind:open={groupDialogOpen}
	{organizationId}
	group={null}
	servers={groupCandidates}
	preselected={groupPreselected}
	onSaved={() => {
		selection = [];
		void loadGroups();
	}} />

<ServerMaintenanceDialog
	bind:open={maintenanceOpen}
	{organizationId}
	server={maintenanceServer}
	onDone={() => void refresh()} />

<RenameServerDialog
	bind:open={renameOpen}
	{organizationId}
	server={renameTarget}
	onRenamed={() => void refresh()} />

<ServerTerminalDialog
	open={consoleServer !== null}
	{organizationId}
	server={consoleServer}
	onOpenChange={(next) => {
		if (!next) consoleServer = null;
	}} />

<Dialog.Root
	open={Boolean(testResult)}
	onOpenChange={(next) => {
		if (!next) testResult = null;
	}}>
	<Dialog.Content class="sm:max-w-2xl">
		<Dialog.Header>
			<Dialog.Title
				>{t('organizations.servers.test_deploy', {
					name: testResult?.server.name ?? '',
				})}</Dialog.Title>
			<Dialog.Description>
				litepod ran <code class="font-mono">{testResult?.result.image}</code> as a throwaway container
				over SSH. This server's state is unchanged either way.
			</Dialog.Description>
		</Dialog.Header>
		{#if testResult}
			<div class="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
				<Badge variant={testResult.result.success ? 'secondary' : 'destructive'}>
					{testResult.result.success
						? t('organizations.servers.container_ran')
						: t('organizations.servers.failed')}
				</Badge>
				<span>{t('organizations.servers.exit_code', { code: testResult.result.exit_code })}</span>
				<span>·</span>
				<span>{(testResult.result.duration_ms / 1000).toFixed(1)}s</span>
			</div>
			{#if testResult.result.stdout.trim()}
				<div class="space-y-1">
					<p class="text-xs font-medium">{t('organizations.servers.output')}</p>
					<pre
						class="max-h-72 overflow-auto rounded-lg border bg-muted/40 p-3 text-xs whitespace-pre-wrap">{testResult.result.stdout.trim()}</pre>
				</div>
			{/if}
			{#if testResult.result.stderr.trim()}
				<div class="space-y-1">
					<p class="text-xs font-medium">
						{testResult.result.success
							? t('organizations.servers.pull_output')
							: t('organizations.servers.errors')}
					</p>
					<pre
						class="max-h-72 overflow-auto rounded-lg border bg-muted/40 p-3 text-xs whitespace-pre-wrap">{testResult.result.stderr.trim()}</pre>
				</div>
			{/if}
			{#if !testResult.result.stdout.trim() && !testResult.result.stderr.trim()}
				<p class="text-sm text-muted-foreground">{t('organizations.servers.no_output')}</p>
			{/if}
		{/if}
		<Dialog.Footer>
			<Button variant="outline" onclick={() => (testResult = null)}
				>{t('organizations.common.close')}</Button>
		</Dialog.Footer>
	</Dialog.Content>
</Dialog.Root>

<AlertDialog.Root
	open={Boolean(pendingDelete)}
	onOpenChange={(next) => {
		if (!next) {
			pendingDelete = null;
			pendingDeleteApplications = null;
		}
	}}>
	<AlertDialog.Content>
		<AlertDialog.Header>
			<AlertDialog.Title
				>{t('organizations.servers.remove_confirm', {
					name: pendingDelete?.name ?? '',
				})}</AlertDialog.Title>
			<AlertDialog.Description>
				litepod forgets this server, its stored SSH key, and its setup history. Nothing is
				uninstalled on the host itself — remove the key from its <code class="font-mono"
					>authorized_keys</code> if you want to cut access completely.
			</AlertDialog.Description>
		</AlertDialog.Header>
		{#if blockingApplications.length > 0}
			<div class="space-y-2 rounded-lg border border-destructive/30 bg-destructive/5 p-3">
				<p class="text-sm font-medium">
					{blockingApplications.length} application{blockingApplications.length === 1 ? '' : 's'} still
					deploy here
				</p>
				<p class="text-xs text-muted-foreground">
					Move them to another server or delete them first — removing the server would leave them
					with nowhere to deploy.
				</p>
				<ul class="max-h-48 space-y-1 overflow-auto text-sm">
					{#each blockingApplications as application (application.id)}
						<li>
							<!-- A new tab keeps the removal dialog open, so the user can work through the
							     list without losing their place. -->
							<a
								class="flex items-center gap-1.5 rounded px-1 py-0.5 hover:bg-accent/50 hover:underline"
								href={`${environmentApplicationsHref(application.project_id, application.environment_name)}/${encodeURIComponent(application.id)}`}
								target="_blank"
								rel="noopener noreferrer">
								<span class="truncate">{application.name}</span>
								<span class="truncate text-xs text-muted-foreground">
									· {application.project_name} / {application.environment_name}
								</span>
								<ExternalLinkIcon class="size-3.5 shrink-0 text-muted-foreground" />
							</a>
						</li>
					{/each}
				</ul>
			</div>
		{/if}
		<AlertDialog.Footer>
			<AlertDialog.Cancel>{t('organizations.common.cancel')}</AlertDialog.Cancel>
			<DeleteConfirmControl
				label={t('organizations.servers.remove')}
				pendingLabel={t('organizations.servers.removing')}
				disabled={pendingDeleteApplications === null || blockingApplications.length > 0}
				onconfirm={confirmDelete} />
		</AlertDialog.Footer>
	</AlertDialog.Content>
</AlertDialog.Root>
