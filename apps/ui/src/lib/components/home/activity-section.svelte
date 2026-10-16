<script lang="ts">
	import { onMount } from 'svelte';
	import ArrowRightIcon from '@lucide/svelte/icons/arrow-right';
	import ActivityIcon from '@lucide/svelte/icons/activity';
	import { Button } from '#lib/components/ui/button/index.js';
	import {
		listDeploymentQueue,
		streamDeploymentQueueEvents,
		type DeploymentJobDto,
	} from '#lib/clients/project-client.js';
	import ActivityFeedItem from './activity-feed-item.svelte';
	import SkeletonRows from '#lib/components/custom/skeleton-rows.svelte';
	import { allowed } from '#lib/state/permissions.svelte.js';
	import { t } from '#lib/i18n/index.svelte.js';

	let { organizationId, limit = 12 }: { organizationId: string; limit?: number } = $props();

	let jobs = $state<DeploymentJobDto[]>([]);
	let loading = $state(true);
	let loadError = $state('');
	let streamState = $state<'live' | 'reconnecting'>('live');

	async function refresh() {
		try {
			jobs = (await listDeploymentQueue(organizationId, limit)).slice(0, limit);
			loadError = '';
		} catch (error) {
			loadError = error instanceof Error ? error.message : t('common.home.activity_load_failed');
		} finally {
			loading = false;
		}
	}

	onMount(() => {
		void refresh();
		const events = streamDeploymentQueueEvents();
		const update = () => void refresh();
		events.addEventListener('status', update);
		events.addEventListener('terminal', update);
		events.onopen = () => (streamState = 'live');
		events.onerror = () => {
			if (events.readyState !== EventSource.CLOSED) streamState = 'reconnecting';
		};
		return () => {
			events.removeEventListener('status', update);
			events.removeEventListener('terminal', update);
			events.close();
		};
	});
</script>

<section class="lp-raised overflow-hidden rounded-2xl border bg-card">
	<header class="lp-groove-b flex items-center justify-between gap-3 px-4 py-3">
		<div class="flex min-w-0 items-center gap-2">
			<h2 class="shrink-0 text-[15px] font-semibold tracking-tight">
				{t('common.home.recent_activity')}
			</h2>
			{#if streamState === 'reconnecting'}
				<span class="truncate text-xs text-muted-foreground" title={t('common.home.reconnecting')}
					>{t('common.home.reconnecting')}</span>
			{/if}
		</div>
		<a
			class="inline-flex shrink-0 items-center gap-1 text-[12.5px] text-muted-foreground transition-colors hover:text-foreground"
			href={`/organizations/${encodeURIComponent(organizationId)}/deployment-queue`}>
			{t('common.home.view_full_queue')}
			<ArrowRightIcon class="size-3.5" />
		</a>
	</header>

	{#if loading}
		<div class="p-3"><SkeletonRows rows={4} height="h-10" /></div>
	{:else if loadError}
		<div class="p-6 text-sm text-destructive">
			{loadError}
			<Button variant="outline" size="sm" class="mt-3 block" onclick={refresh}
				>{t('common.home.retry')}</Button>
		</div>
	{:else if jobs.length === 0}
		<div class="flex min-h-32 flex-col items-center justify-center gap-2 p-8 text-center">
			<ActivityIcon class="size-6 text-muted-foreground" />
			<h3 class="font-medium">{t('common.home.no_deployments')}</h3>
			<p class="text-sm text-muted-foreground">{t('common.home.no_deployments_hint')}</p>
			{#if allowed('project.create')}
				<Button href="/projects" size="sm" class="mt-1"
					>{t('common.home.deploy_first_application')}</Button>
			{/if}
		</div>
	{:else}
		<ol class="flex flex-col p-1.5">
			{#each jobs as job (job.id)}
				<ActivityFeedItem {job} />
			{/each}
		</ol>
	{/if}
</section>
