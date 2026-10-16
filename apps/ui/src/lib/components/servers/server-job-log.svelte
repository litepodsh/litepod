<script lang="ts">
	import CheckIcon from '@lucide/svelte/icons/check';
	import LoaderCircleIcon from '@lucide/svelte/icons/loader-circle';
	import XIcon from '@lucide/svelte/icons/x';
	import {
		getServer,
		listServerJobs,
		type Server,
		type ServerJob,
	} from '#lib/clients/server-client.js';
	import { t } from '#lib/i18n/index.svelte.js';

	let {
		organizationId,
		serverId,
		pollMs = 2000,
		onFinished,
	}: {
		organizationId: string;
		serverId: string;
		pollMs?: number;
		/** Fired once when the job stops running, with the refreshed server row. */
		onFinished?: (job: ServerJob, server: Server) => void;
	} = $props();

	// The first poll runs on mount, so the queued job appears without waiting for an interval.
	let job = $state<ServerJob | null>(null);
	let error = $state('');
	let logElement = $state<HTMLPreElement | null>(null);

	const running = $derived(!job || job.status === 'queued' || job.status === 'running');

	// `list_jobs` orders by created_at DESC, so the newest job is the one this run started.
	async function poll(): Promise<boolean> {
		const jobs = await listServerJobs(organizationId, serverId);
		job = jobs[0] ?? job;
		if (job && (job.status === 'succeeded' || job.status === 'failed')) {
			onFinished?.(job, await getServer(organizationId, serverId));
			return true;
		}
		return false;
	}

	$effect(() => {
		if (!serverId) return;
		let stopped = false;
		let timer: ReturnType<typeof setTimeout>;

		const tick = async () => {
			try {
				if (await poll()) return;
			} catch (caught) {
				error =
					caught instanceof Error ? caught.message : t('organizations.servers.setup_contact_lost');
			}
			if (!stopped) timer = setTimeout(() => void tick(), pollMs);
		};
		void tick();

		return () => {
			stopped = true;
			clearTimeout(timer);
		};
	});

	// Follow the tail the way a terminal does, so long installs stay readable.
	$effect(() => {
		const text = job?.log;
		if (logElement && text !== undefined) logElement.scrollTop = logElement.scrollHeight;
	});
</script>

<div class="space-y-3">
	<div class="flex items-center gap-2 text-sm">
		{#if running}
			<LoaderCircleIcon class="size-4 animate-spin text-muted-foreground" />
			<span class="font-medium"
				>{job?.phase?.replaceAll('-', ' ') ?? t('organizations.servers.queued')}</span>
		{:else if job?.status === 'succeeded'}
			<CheckIcon class="size-4 text-success" />
			<span class="font-medium">{t('organizations.servers.setup_finished')}</span>
		{:else}
			<XIcon class="size-4 text-destructive" />
			<span class="font-medium text-destructive">{t('organizations.servers.setup_failed')}</span>
		{/if}
	</div>

	<pre
		bind:this={logElement}
		class="max-h-64 min-h-32 overflow-auto rounded-md border bg-muted/50 p-3 font-mono text-xs whitespace-pre-wrap">{job?.log?.trim() ||
			t('organizations.servers.setup_waiting')}</pre>

	{#if job?.error}
		<p
			class="rounded-lg border border-destructive/30 bg-destructive/5 p-3 text-sm text-destructive">
			{job.error.message}
		</p>
	{/if}
	{#if error}
		<p class="text-sm text-muted-foreground">{error}</p>
	{/if}
</div>
