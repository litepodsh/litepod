<script lang="ts">
	import { allowedHere } from '#lib/state/permissions.svelte.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import DeploymentLogViewer from '#lib/components/projects/deployment-log-viewer.svelte';
	import OsIcon from '#lib/components/os-icon.svelte';
	import LiveTimer from '#lib/components/live-timer.svelte';
	import { formatDuration } from '#lib/utils/format-duration.js';
	import { t } from '#lib/i18n/index.svelte.js';
	import type {
		DeploymentRunLogDto,
		DeploymentRunReplicaDto,
	} from '#lib/clients/project-client.js';
	import CircleCheckIcon from '@lucide/svelte/icons/circle-check';
	import CircleXIcon from '@lucide/svelte/icons/circle-x';
	import LoaderCircleIcon from '@lucide/svelte/icons/loader-circle';
	import ClockIcon from '@lucide/svelte/icons/clock';
	import BanIcon from '@lucide/svelte/icons/ban';
	import RotateCwIcon from '@lucide/svelte/icons/rotate-cw';
	import ServerIcon from '@lucide/svelte/icons/server';

	const ALL = '__all__';

	let {
		replicas,
		logs,
		onRetry,
		onRefresh,
		livePaused,
		onToggleLive,
		onAskAssistant,
	}: {
		replicas: DeploymentRunReplicaDto[];
		logs: DeploymentRunLogDto[];
		/** Re-deploys one server. Omitted while the run is still in flight. */
		onRetry?: (serverId: string) => Promise<void>;
		onRefresh?: () => void | Promise<void>;
		livePaused?: boolean;
		onToggleLive?: () => void | Promise<void>;
		onAskAssistant?: (text: string) => void;
	} = $props();

	// Problems first: a failed or still-running server is what the user came to look at.
	const ORDER: Record<DeploymentRunReplicaDto['status'], number> = {
		failed: 0,
		deploying: 1,
		queued: 2,
		canceled: 3,
		succeeded: 4,
	};
	const sorted = $derived(
		[...replicas].sort(
			(a, b) => ORDER[a.status] - ORDER[b.status] || a.server_name.localeCompare(b.server_name),
		),
	);
	const succeeded = $derived(replicas.filter((r) => r.status === 'succeeded').length);
	const failed = $derived(replicas.filter((r) => r.status === 'failed').length);

	let selectedId = $state<string | null>(null);
	let retrying = $state<string | null>(null);

	// Pick a sensible default once replicas arrive, and recover if the selection vanished.
	$effect(() => {
		if (sorted.length === 0) return;
		if (selectedId !== ALL && !sorted.some((r) => r.server_id === selectedId)) {
			selectedId = sorted[0].server_id;
		}
	});

	const byId = (id: string | null) => replicas.find((r) => r.server_id === id) ?? null;
	const nameOf = (id: string | null | undefined) => (id ? (byId(id)?.server_name ?? id) : '');

	function logsFor(id: string | null): DeploymentRunLogDto[] {
		if (id === ALL) {
			return logs.map((log) =>
				log.server_id ? { ...log, message: `[${nameOf(log.server_id)}] ${log.message}` } : log,
			);
		}
		return logs.filter((log) => log.server_id === id);
	}

	async function retry(serverId: string) {
		if (!onRetry) return;
		retrying = serverId;
		try {
			await onRetry(serverId);
		} finally {
			retrying = null;
		}
	}

	function duration(replica: DeploymentRunReplicaDto): number | null {
		if (replica.started_at && replica.finished_at) return replica.finished_at - replica.started_at;
		return null;
	}
</script>

{#snippet statusIcon(status: DeploymentRunReplicaDto['status'], cls = 'size-4')}
	{#if status === 'succeeded'}
		<CircleCheckIcon class="{cls} text-success" />
	{:else if status === 'failed'}
		<CircleXIcon class="{cls} text-destructive" />
	{:else if status === 'deploying'}
		<LoaderCircleIcon class="{cls} animate-spin text-primary" />
	{:else if status === 'canceled'}
		<BanIcon class="{cls} text-muted-foreground" />
	{:else}
		<ClockIcon class="{cls} text-muted-foreground" />
	{/if}
{/snippet}

{#snippet panel(id: string | null)}
	{@const replica = byId(id)}
	<div class="min-w-0">
		<div class="mb-2 flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 text-sm">
			{#if replica}
				{@render statusIcon(replica.status)}
				<span class="truncate font-medium">{replica.server_name}</span>
				{#if replica.address}<span class="font-mono text-xs text-muted-foreground"
						>{replica.address}</span
					>{/if}
				<span class="text-xs text-muted-foreground">
					{#if duration(replica) !== null}{formatDuration(
							duration(replica)!,
						)}{:else if replica.status === 'deploying' && replica.started_at}<LiveTimer
							startedAt={replica.started_at} />{/if}
				</span>
				{#if replica.status === 'failed' && onRetry && allowedHere('app.deploy')}
					<Button
						size="sm"
						variant="outline"
						class="ml-auto"
						disabled={retrying !== null}
						onclick={() => void retry(replica.server_id)}
						><RotateCwIcon class={retrying === replica.server_id ? 'animate-spin' : ''} />{t(
							'projects.deployment_replicas.retry',
							{ name: replica.server_name },
						)}</Button>
				{/if}
			{:else}
				<ServerIcon class="size-4 text-muted-foreground" />
				<span class="font-medium">{t('projects.deployment_replicas.all_servers')}</span>
			{/if}
		</div>
		{#if replica?.last_error}
			<p class="mb-2 rounded-lg bg-destructive/10 p-2 text-xs break-words text-destructive">
				{replica.last_error}
			</p>
		{/if}
		{#key id}
			{#if logsFor(id).length}
				<DeploymentLogViewer
					logs={logsFor(id)}
					label={t('projects.application_deployment.deployment_output_label')}
					showLineNumbers={false}
					{onRefresh}
					{livePaused}
					{onToggleLive}
					{onAskAssistant} />
			{:else}
				<div
					class="flex min-h-40 items-center justify-center rounded-lg border border-dashed text-sm text-muted-foreground">
					{replica?.status === 'queued'
						? t('projects.deployment_replicas.waiting_turn')
						: t('projects.application_deployment.waiting_output')}
				</div>
			{/if}
		{/key}
	</div>
{/snippet}

<div class="grid min-w-0 gap-3 lg:grid-cols-[15rem_minmax(0,1fr)]">
	<div class="min-w-0">
		<p class="mb-2 text-xs text-muted-foreground">
			{t('projects.deployment_replicas.summary', {
				succeeded,
				failed,
				total: replicas.length,
			})}
		</p>
		<ul class="flex gap-1.5 overflow-x-auto pb-1 lg:max-h-[32rem] lg:flex-col lg:overflow-y-auto">
			{#each sorted as replica (replica.server_id)}
				{@const active = selectedId === replica.server_id}
				<li class="shrink-0 lg:shrink">
					<button
						type="button"
						class="flex w-full min-w-44 items-center gap-2 rounded-lg border px-2.5 py-2 text-left transition hover:bg-muted/50 {active
							? 'border-primary bg-primary/5'
							: ''}"
						onclick={() => (selectedId = replica.server_id)}>
						{@render statusIcon(replica.status)}
						<span class="shrink-0 text-muted-foreground"
							><OsIcon os={replica.os} distribution={replica.distribution} class="size-4" /></span>
						<span class="min-w-0 flex-1">
							<span class="block truncate text-sm font-medium">{replica.server_name}</span>
							{#if replica.address}<span
									class="block truncate font-mono text-[11px] text-muted-foreground"
									>{replica.address}</span
								>{/if}
						</span>
					</button>
				</li>
			{/each}
			<li class="shrink-0 lg:shrink">
				<button
					type="button"
					class="flex w-full min-w-44 items-center gap-2 rounded-lg border px-2.5 py-2 text-left text-sm transition hover:bg-muted/50 {selectedId ===
					ALL
						? 'border-primary bg-primary/5'
						: ''}"
					onclick={() => (selectedId = ALL)}>
					<ServerIcon class="size-4 text-muted-foreground" />
					{t('projects.deployment_replicas.all_servers')}
				</button>
			</li>
		</ul>
	</div>

	<div class="min-w-0">
		{@render panel(selectedId)}
	</div>
</div>
