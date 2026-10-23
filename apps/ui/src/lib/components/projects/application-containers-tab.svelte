<script lang="ts">
	import { allowedHere } from '#lib/state/permissions.svelte.js';
	import SkeletonRows from '#lib/components/custom/skeleton-rows.svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Card from '#lib/components/ui/card/index.js';
	import * as Collapsible from '#lib/components/ui/collapsible/index.js';
	import OsIcon from '#lib/components/os-icon.svelte';
	import * as Sheet from '#lib/components/ui/sheet/index.js';
	import { toast } from '#lib/utils/toast.js';
	import { t } from '#lib/i18n/index.svelte.js';
	import { containerStatusTone, isRunning } from '#lib/utils/container-status.js';
	import { toneBadge } from '#lib/utils/status-tone.js';
	import {
		formatCpu,
		formatMemory,
		type ContainerMetric,
	} from '#lib/clients/container-metrics-client.js';
	import {
		containerAction,
		fetchDeploymentLogs,
		inspectApplicationContainer,
		listReplicaContainers,
		type ReplicaContainersDto,
		type ContainerStatusDto,
		type ComposeServiceStatus,
		type ContainerAction,
	} from '#lib/clients/project-client.js';
	import PlayIcon from '@lucide/svelte/icons/play';
	import SquareIcon from '@lucide/svelte/icons/square';
	import RotateCwIcon from '@lucide/svelte/icons/rotate-cw';
	import FileTextIcon from '@lucide/svelte/icons/file-text';
	import CopyIcon from '@lucide/svelte/icons/copy';
import InfoIcon from '@lucide/svelte/icons/info';
	import RefreshCwIcon from '@lucide/svelte/icons/refresh-cw';
	import TerminalIcon from '@lucide/svelte/icons/terminal';
	import ChevronRightIcon from '@lucide/svelte/icons/chevron-right';

	type Row = {
		containerName: string;
		service: string | null;
		status: string;
		ports: string[];
		/** Release-independent `http://host[:port]` other containers can dial. */
		internalUrl?: string;
		/** Replica server for a group-targeted application; absent otherwise. */
		serverId?: string;
	};

	/** Inspect results are per container *and* server: replicas share container names. */
	const containerKey = (row: Pick<Row, 'containerName' | 'serverId'>) =>
		row.serverId ? `${row.serverId}/${row.containerName}` : row.containerName;

	let {
		containerStatus,
		composeServices = [],
		containerMetrics = new Map<string, ContainerMetric>(),
		deployType,
		busy = false,
		isCompose = false,
		isGroup = false,
		organizationId,
		projectId,
		environmentId,
		applicationId,
		onRefresh,
		onComposeStart,
		onComposeStop,
		onOpenTerminal,
	}: {
		containerStatus: ContainerStatusDto | null;
		composeServices?: ComposeServiceStatus[];
		containerMetrics?: Map<string, ContainerMetric>;
		deployType: string | undefined;
		busy?: boolean;
		isCompose?: boolean;
		/** Group-targeted application: containers are listed per replica server. */
		isGroup?: boolean;
		organizationId: string;
		projectId: string;
		environmentId: string;
		applicationId: string;
		onRefresh: () => Promise<void>;
		onComposeStart: () => Promise<void>;
		onComposeStop: () => Promise<void>;
		onOpenTerminal: (containerName: string) => void;
	} = $props();

	const rows = $derived<Row[]>(
		isCompose
			? composeServices.map((s) => ({
					containerName: s.container_name,
					service: s.service,
					status: s.status,
					ports: s.ports ?? [],
				}))
			: containerStatus?.container_name
				? [
						{
							containerName: containerStatus.container_name,
							service: null,
							status: containerStatus.status,
							ports: [],
							internalUrl: containerStatus.internal_host
								? `http://${containerStatus.internal_host}${containerStatus.internal_port ? `:${containerStatus.internal_port}` : ''}`
								: undefined,
						},
					]
				: [],
	);

	let replicas = $state<ReplicaContainersDto[]>([]);
	let replicasLoading = $state(false);
	let replicaBusy = $state<Record<string, boolean>>({});
	let openReplicas = $state<Record<string, boolean>>({});

	const replicaRows = (replica: ReplicaContainersDto): Row[] =>
		replica.containers.map((c) => ({
			containerName: c.container_name,
			service: c.service,
			status: c.status,
			ports: c.ports,
			serverId: replica.server_id,
		}));

	const groupTotal = $derived(replicas.reduce((n, r) => n + r.containers.length, 0));
	const groupRunning = $derived(
		replicas.reduce((n, r) => n + r.containers.filter((c) => isRunning(c.status)).length, 0),
	);
	const runningCount = $derived(
		isGroup ? groupRunning : rows.filter((r) => isRunning(r.status)).length,
	);

	async function loadReplicas() {
		replicasLoading = true;
		try {
			replicas = await listReplicaContainers(
				organizationId,
				projectId,
				environmentId,
				applicationId,
			);
		} catch (err) {
			toast.error(
				err instanceof Error
					? err.message
					: t('projects.application_containers.action_failed', {
							action: 'list',
						}),
			);
		} finally {
			replicasLoading = false;
		}
	}

	$effect(() => {
		if (isGroup) void loadReplicas();
	});

	async function refreshAll() {
		await Promise.all([onRefresh(), isGroup ? loadReplicas() : Promise.resolve()]);
	}

	/** Runs `action` on every container of one replica that it applies to. */
	async function runReplicaAction(replica: ReplicaContainersDto, action: ContainerAction) {
		const targets = replicaRows(replica).filter((row) =>
			action === 'start' ? !isRunning(row.status) : isRunning(row.status),
		);
		if (targets.length === 0) return;
		replicaBusy = { ...replicaBusy, [replica.server_id]: true };
		try {
			const results = await Promise.allSettled(
				targets.map((row) =>
					containerAction(
						organizationId,
						projectId,
						environmentId,
						applicationId,
						row.containerName,
						action,
						row.serverId,
					),
				),
			);
			const failed = results.filter((r) => r.status === 'rejected').length;
			if (failed > 0) {
				toast.error(
					t('projects.application_containers.group_action_partial', {
						failed,
						total: targets.length,
						action,
					}),
				);
			} else {
				toast.success(t('projects.application_containers.action_requested', { action }));
			}
			await loadReplicas();
		} finally {
			replicaBusy = { ...replicaBusy, [replica.server_id]: false };
		}
	}

	const inspectByContainer = $state(new Map<string, Record<string, unknown>>());
	let inspectLoading = $state<Record<string, boolean>>({});
	let logsFor = $state<string | null>(null);
	let logsContent = $state('');
	let logsLoading = $state(false);
	let inspectFor = $state<Row | null>(null);

	async function ensureInspect(row: Row) {
		const key = containerKey(row);
		if (inspectByContainer.has(key)) return;
		inspectLoading = { ...inspectLoading, [key]: true };
		try {
			const data = await inspectApplicationContainer(
				organizationId,
				projectId,
				environmentId,
				applicationId,
				row.containerName,
				row.serverId,
			);
			inspectByContainer.set(key, data);
		} catch (err) {
			toast.error(
				err instanceof Error ? err.message : t('projects.application_containers.inspect_failed'),
			);
		} finally {
			inspectLoading = { ...inspectLoading, [key]: false };
		}
	}

	async function openLogs(row: Row) {
		logsFor = row.containerName;
		logsContent = '';
		logsLoading = true;
		try {
			const result = await fetchDeploymentLogs(
				organizationId,
				projectId,
				environmentId,
				applicationId,
				200,
				row.containerName,
				row.serverId,
			);
			logsContent = result.lines
				.map((line) => (line.ts ? `${line.ts} ${line.text}` : line.text))
				.join('\n');
		} catch (err) {
			toast.error(
				err instanceof Error ? err.message : t('projects.application_containers.logs_failed'),
			);
		} finally {
			logsLoading = false;
		}
	}

	async function openInspect(row: Row) {
		inspectFor = row;
		await ensureInspect(row);
	}

	function stateClass(status: string) {
		return toneBadge(containerStatusTone(status));
	}

	async function runContainerAction(row: Row, action: ContainerAction) {
		try {
			await containerAction(
				organizationId,
				projectId,
				environmentId,
				applicationId,
				row.containerName,
				action,
				row.serverId,
			);
			toast.success(t('projects.application_containers.action_requested', { action }));
			if (row.serverId) await loadReplicas();
			else await onRefresh();
		} catch (err) {
			toast.error(
				err instanceof Error
					? err.message
					: t('projects.application_containers.action_failed', { action }),
			);
		}
	}

	function imageOf(detail: Record<string, unknown> | undefined): string {
		if (!detail) return '';
		const cfg = (detail.Config ?? detail.config) as Record<string, unknown> | undefined;
		const img = cfg?.Image ?? cfg?.image;
		if (typeof img === 'string') return img;
		const created = (detail.Image ?? detail.image) as string | undefined;
		return typeof created === 'string' ? created : '';
	}

	function copyInternalUrl(url: string) {
		navigator.clipboard
			.writeText(url)
			.then(() => toast.success(t('projects.application_containers.internal_url_copied')))
			.catch(() => toast.error(t('projects.application_containers.internal_url_copy_failed')));
	}

	function numberField(detail: Record<string, unknown>, keys: string[]): number | null {
		for (const key of keys) {
			const value = detail[key];
			if (typeof value === 'number' && Number.isFinite(value)) return value;
			if (typeof value === 'string' && value.trim() !== '') {
				const parsed = Number(value);
				if (Number.isFinite(parsed)) return parsed;
			}
		}
		return null;
	}

	function diskSizeOf(detail: Record<string, unknown> | undefined): number | null {
		if (!detail) return null;
		return numberField(detail, ['SizeRootFs', 'size_root_fs', 'Size', 'size', 'SizeRw', 'size_rw']);
	}

	function formatBytes(bytes: number): string {
		const units = ['B', 'KiB', 'MiB', 'GiB', 'TiB'];
		let value = bytes;
		let unit = 0;
		while (value >= 1024 && unit < units.length - 1) {
			value /= 1024;
			unit += 1;
		}
		const rounded = value >= 10 || Number.isInteger(value) ? value.toFixed(0) : value.toFixed(1);
		return `${rounded} ${units[unit]}`;
	}
</script>

{#snippet containerRow(row: Row)}
	{@const metric = row.serverId ? undefined : containerMetrics.get(row.containerName)}
	{@const inspect = inspectByContainer.get(containerKey(row))}
	{@const isUp = isRunning(row.status)}
	<li class="rounded-xl border p-4">
		<div class="flex flex-wrap items-start justify-between gap-3">
			<div class="min-w-0 flex-1">
				<div class="flex flex-wrap items-center gap-2">
					{#if row.service}
						<span class="font-medium">{row.service}</span>
						<span class="text-xs text-muted-foreground">·</span>
					{/if}
					<span class="truncate font-mono text-sm">{row.containerName}</span>
					<span
						class="rounded-full border px-2 py-0.5 text-[11px] font-medium capitalize {stateClass(
							row.status,
						)}">{row.status}</span>
				</div>
				<div class="mt-1 flex flex-wrap gap-x-3 gap-y-1 text-xs text-muted-foreground">
					{#if inspect}
						{#if imageOf(inspect)}<span class="truncate"
								>{t('projects.application_containers.image_label', {
									name: imageOf(inspect),
								})}</span
							>{/if}
						{#if row.ports.length > 0}
							<span
								>{t('projects.application_containers.ports_label', {
									ports: row.ports.join(', '),
								})}</span>
						{/if}
						{#if diskSizeOf(inspect) !== null}
							<span
								>{t('projects.application_containers.disk_label', {
									size: formatBytes(diskSizeOf(inspect)!),
								})}</span>
						{/if}
					{:else if row.ports.length > 0}
						<span
							>{t('projects.application_containers.ports_label', {
								ports: row.ports.join(', '),
							})}</span>
					{:else}
						<span>{t('projects.application_containers.inspect_hint')}</span>
					{/if}
					{#if row.internalUrl}
						<button
							type="button"
							class="inline-flex items-center gap-1 rounded font-mono hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
							title={t('projects.application_containers.internal_url_label')}
							onclick={() => copyInternalUrl(row.internalUrl!)}
							><CopyIcon class="size-3" />{row.internalUrl}</button>
					{/if}
					{#if metric && metric.cpu_percent !== null}
						<span>CPU {formatCpu(metric.cpu_percent)}</span>
						<span>RAM {formatMemory(metric.memory_usage_bytes, metric.memory_limit_bytes)}</span>
					{/if}
				</div>
			</div>
			<div class="flex flex-wrap gap-2">
				{#if allowedHere('app.deploy')}
					<Button
						size="sm"
						variant="outline"
						disabled={busy || isUp}
						onclick={() => void runContainerAction(row, 'start')}
						><PlayIcon />{t('projects.environment.start')}</Button>
					<Button
						size="sm"
						variant="outline"
						disabled={busy || !isUp}
						onclick={() => void runContainerAction(row, 'stop')}
						><SquareIcon />{t('projects.environment.stop')}</Button>
					<Button
						size="sm"
						variant="outline"
						disabled={busy || !isUp}
						onclick={() => void runContainerAction(row, 'restart')}
						><RotateCwIcon />{t('projects.application_containers.restart')}</Button>
				{/if}
				<Button size="sm" variant="outline" onclick={() => void openLogs(row)}
					><FileTextIcon />{t('projects.application.logs')}</Button>
				<Button size="sm" variant="outline" onclick={() => void openInspect(row)}
					><InfoIcon />{t('projects.application_containers.inspect')}</Button>
				<!-- The terminal session resolves the app's single server, so a replica has none. -->
				{#if !row.serverId}
					<Button
						size="sm"
						variant="outline"
						disabled={!isUp}
						onclick={() => onOpenTerminal(row.containerName)}
						><TerminalIcon />{t('projects.application.terminal')}</Button>
				{/if}
			</div>
		</div>
	</li>
{/snippet}

<Card.Root>
	<Card.Header>
		<div class="flex flex-wrap items-center justify-between gap-3">
			<div>
				<Card.Title>{t('projects.application.containers')}</Card.Title>
				<Card.Description>
					{#if isGroup}
						{t('projects.application_containers.group_summary', {
							running: groupRunning,
							total: groupTotal,
							servers: replicas.length,
						})}
					{:else}
						{t('projects.application_containers.summary', {
							running: runningCount,
							total: rows.length,
						})}
					{/if}
				</Card.Description>
			</div>
			<div class="flex flex-wrap gap-2">
				{#if isCompose && allowedHere('app.deploy')}
					{@const anyRunning = runningCount > 0}
					<Button
						variant="outline"
						disabled={busy || anyRunning}
						onclick={() => void onComposeStart()}
						><PlayIcon />{t('projects.application_containers.start_all')}</Button>
					<Button
						variant="outline"
						disabled={busy || !anyRunning}
						onclick={() => void onComposeStop()}
						><SquareIcon />{t('projects.application_containers.stop_all')}</Button>
				{/if}
				<Button
					variant="outline"
					disabled={busy || replicasLoading}
					onclick={() => void refreshAll()}
					><RefreshCwIcon class={busy || replicasLoading ? 'animate-spin' : ''} />{t(
						'projects.application_containers.refresh',
					)}</Button>
			</div>
		</div>
	</Card.Header>
	<Card.Content>
		{#if isGroup}
			{#if replicasLoading && replicas.length === 0}
				<SkeletonRows
					rows={2}
					height="h-14"
					label={t('projects.application_containers.loading_details')} />
			{:else if replicas.length === 0}
				<div class="rounded-2xl border border-dashed p-8 text-center text-muted-foreground">
					{deployType === 'compose'
						? t('projects.application_containers.empty_compose')
						: t('projects.application_containers.empty_container')}
				</div>
			{:else}
				<ul class="grid gap-3">
					{#each replicas as replica (replica.server_id)}
						{@const replicaRunning = replica.containers.filter((c) => isRunning(c.status)).length}
						{@const replicaTotal = replica.containers.length}
						{@const rBusy = busy || !!replicaBusy[replica.server_id]}
						<li>
							<Collapsible.Root
								bind:open={
									() => openReplicas[replica.server_id] ?? false,
									(v) => (openReplicas = { ...openReplicas, [replica.server_id]: v })
								}
								class="rounded-xl border">
								<div class="flex flex-wrap items-center justify-between gap-3 p-3">
									<Collapsible.Trigger
										class="group flex min-w-0 flex-1 items-center gap-3 rounded-lg text-left outline-none focus-visible:ring-2 focus-visible:ring-ring">
										<ChevronRightIcon
											class="size-4 shrink-0 text-muted-foreground transition-transform group-data-[state=open]:rotate-90" />
										<span
											class="flex size-8 shrink-0 items-center justify-center rounded-lg bg-muted text-foreground"
											><OsIcon
												os={replica.os}
												distribution={replica.distribution}
												class="size-4" /></span>
										<span class="min-w-0">
											<span class="flex flex-wrap items-center gap-2">
												<span class="truncate font-medium">{replica.server_name}</span>
												<span
													class="rounded-full border px-2 py-0.5 text-[11px] font-medium capitalize {stateClass(
														replica.reachable ? replica.replica_status : 'unavailable',
													)}"
													>{replica.reachable
														? replica.replica_status.replaceAll('_', ' ')
														: 'unreachable'}</span>
											</span>
											<span class="flex flex-wrap gap-x-3 text-xs text-muted-foreground">
												{#if replica.address}<span class="font-mono">{replica.address}</span>{/if}
												<span
													>{t('projects.application_containers.replica_summary', {
														running: replicaRunning,
														total: replicaTotal,
													})}</span>
											</span>
										</span>
									</Collapsible.Trigger>
									{#if allowedHere('app.deploy') && replicaTotal > 0}
										<div class="flex flex-wrap gap-2">
											<Button
												size="sm"
												variant="outline"
												disabled={rBusy || replicaRunning === replicaTotal}
												onclick={() => void runReplicaAction(replica, 'start')}
												><PlayIcon />{t('projects.application_containers.start_all')}</Button>
											<Button
												size="sm"
												variant="outline"
												disabled={rBusy || replicaRunning === 0}
												onclick={() => void runReplicaAction(replica, 'stop')}
												><SquareIcon />{t('projects.application_containers.stop_all')}</Button>
											<Button
												size="sm"
												variant="outline"
												disabled={rBusy || replicaRunning === 0}
												onclick={() => void runReplicaAction(replica, 'restart')}
												><RotateCwIcon />{t('projects.application_containers.restart_all')}</Button>
										</div>
									{/if}
								</div>
								<Collapsible.Content>
									<div class="border-t p-3">
										{#if replica.last_error}
											<p class="mb-3 rounded-lg bg-destructive/10 p-2 text-xs text-destructive">
												{replica.last_error}
											</p>
										{/if}
										{#if !replica.reachable}
											<p class="text-sm text-muted-foreground">
												{t('projects.application_containers.replica_unreachable')}
											</p>
										{:else if replicaTotal === 0}
											<p class="text-sm text-muted-foreground">
												{t('projects.application_containers.replica_empty')}
											</p>
										{:else}
											<ul class="grid gap-3">
												{#each replicaRows(replica) as row (row.containerName)}
													{@render containerRow(row)}
												{/each}
											</ul>
										{/if}
									</div>
								</Collapsible.Content>
							</Collapsible.Root>
						</li>
					{/each}
				</ul>
			{/if}
		{:else if rows.length === 0}
			<div class="rounded-2xl border border-dashed p-8 text-center text-muted-foreground">
				{deployType === 'compose'
					? t('projects.application_containers.empty_compose')
					: t('projects.application_containers.empty_container')}
			</div>
		{:else}
			<ul class="grid gap-3">
				{#each rows as row (row.containerName)}
					{@render containerRow(row)}
				{/each}
			</ul>
		{/if}
	</Card.Content>
</Card.Root>

<Sheet.Root
	open={logsFor !== null}
	onOpenChange={(open) => {
		if (!open) logsFor = null;
	}}>
	<Sheet.Content side="right" class="w-full sm:max-w-2xl">
		<Sheet.Header>
			<Sheet.Title
				>{t('projects.application_containers.logs_title', { name: logsFor ?? '' })}</Sheet.Title>
			<Sheet.Description>{t('projects.application_containers.logs_description')}</Sheet.Description>
		</Sheet.Header>
		{#if logsLoading}
			<SkeletonRows
				rows={8}
				height="h-4"
				label={t('projects.application_containers.loading_logs')}
				class="mt-4" />
		{:else}
			<pre class="mt-4 max-h-[70vh] overflow-auto rounded-lg bg-muted p-3 text-xs">{logsContent ||
					t('projects.application_containers.no_logs')}</pre>
		{/if}
	</Sheet.Content>
</Sheet.Root>

<Sheet.Root
	open={inspectFor !== null}
	onOpenChange={(open) => {
		if (!open) inspectFor = null;
	}}>
	<Sheet.Content side="right" class="w-full sm:max-w-2xl">
		<Sheet.Header>
			<Sheet.Title
				>{t('projects.application_containers.inspect_title', {
					name: inspectFor?.containerName ?? '',
				})}</Sheet.Title>
			<Sheet.Description
				>{t('projects.application_containers.inspect_description')}</Sheet.Description>
		</Sheet.Header>
		{#if inspectFor && inspectLoading[containerKey(inspectFor)]}
			<SkeletonRows
				rows={8}
				height="h-4"
				label={t('projects.application_containers.loading_details')}
				class="mt-4" />
		{:else if inspectFor}
			<pre class="mt-4 max-h-[70vh] overflow-auto rounded-lg bg-muted p-3 text-xs">{JSON.stringify(
					inspectByContainer.get(containerKey(inspectFor)) ?? {},
					null,
					2,
				)}</pre>
		{/if}
	</Sheet.Content>
</Sheet.Root>
