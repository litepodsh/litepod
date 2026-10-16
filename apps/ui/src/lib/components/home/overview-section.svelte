<script lang="ts">
	import { onMount } from 'svelte';
	import FolderKanbanIcon from '@lucide/svelte/icons/folder-kanban';
	import PlusIcon from '@lucide/svelte/icons/plus';
	import ArrowRightIcon from '@lucide/svelte/icons/arrow-right';
	import SearchIcon from '@lucide/svelte/icons/search';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { allowed } from '#lib/state/permissions.svelte.js';
	import {
		getApplicationStats,
		type ApplicationDto,
		type ApplicationStatsDto,
		type ProjectDto,
	} from '#lib/clients/project-client.js';
	import { getOrganizationDashboard } from '#lib/clients/dashboard-client.js';
	import { toast } from '#lib/utils/toast.js';
	import OverviewProjectCard from './overview-project-card.svelte';
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import { t } from '#lib/i18n/index.svelte.js';

	let { organizationId }: { organizationId: string } = $props();

	const PROJECT_LIMIT = 6;
	const STATS_POLL_MS = 10_000;

	type AppOverview = {
		application: ApplicationDto;
		deployment: { status: string } | null;
		stats: ApplicationStatsDto | null;
	};

	type ProjectOverview = {
		project: ProjectDto;
		environmentId: string;
		environmentName: string;
		isProduction: boolean;
		apps: AppOverview[];
	};

	let overviews = $state<ProjectOverview[]>([]);
	let query = $state('');
	let visible = $derived.by(() => {
		const needle = query.trim().toLowerCase();
		if (!needle) return overviews;
		return overviews.filter(
			(overview) =>
				overview.project.name.toLowerCase().includes(needle) ||
				overview.apps.some((app) => app.application.name.toLowerCase().includes(needle)),
		);
	});
	let totalProjects = $state(0);
	let loading = $state(true);
	let loadError = $state('');
	let failedApplicationIds = $state<Set<string>>(new Set());

	onMount(() => {
		void load();
		const interval = window.setInterval(() => void pollStats(), STATS_POLL_MS);
		return () => window.clearInterval(interval);
	});

	async function load() {
		loading = true;
		loadError = '';
		try {
			const dashboard = await getOrganizationDashboard(organizationId);
			totalProjects = dashboard.total_projects;
			overviews = dashboard.projects.map(({ project, environment }) => ({
				project,
				environmentId: environment?.id ?? '',
				environmentName: environment?.name ?? '',
				isProduction: environment?.is_production ?? false,
				apps: (environment?.applications ?? []).map(({ application, deployment_status }) => ({
					application,
					deployment: { status: deployment_status },
					stats: null,
				})),
			}));
			void pollStats();
		} catch (error) {
			loadError = error instanceof Error ? error.message : t('common.home.projects_load_failed');
			toast.error(loadError);
		} finally {
			loading = false;
		}
	}

	async function fetchStats(
		projectId: string,
		environmentId: string,
		applicationId: string,
	): Promise<ApplicationStatsDto | null> {
		try {
			return await getApplicationStats(organizationId, projectId, environmentId, applicationId);
		} catch {
			return null;
		}
	}

	async function pollStats() {
		if (overviews.length === 0) return;
		const updated = await Promise.all(
			overviews.map(async (overview) => {
				const apps = await Promise.all(
					overview.apps.map(async (app): Promise<AppOverview> => {
						if (app.deployment?.status !== 'running') return app;
						const stats = await fetchStats(
							overview.project.id,
							overview.environmentId,
							app.application.id,
						);
						return { ...app, stats };
					}),
				);
				return { ...overview, apps };
			}),
		);
		overviews = updated;
	}
</script>

<section class="flex flex-col gap-3">
	<div class="flex flex-wrap items-center justify-between gap-3">
		<h2 class="text-[15px] font-semibold tracking-tight">{t('projects.common.projects')}</h2>
		<div class="flex min-w-0 items-center gap-3 max-sm:w-full">
			{#if overviews.length > 1}
				<label
					class="lp-well relative flex min-w-0 flex-1 items-center rounded-full bg-muted/60 sm:w-60 sm:flex-none">
					<SearchIcon class="pointer-events-none absolute left-3 size-4 text-muted-foreground" />
					<Input
						bind:value={query}
						placeholder={t('common.home.filter_projects')}
						aria-label={t('common.home.filter_projects')}
						class="h-8 rounded-full border-0 bg-transparent pl-9 text-[13px] shadow-none" />
				</label>
			{/if}
			{#if totalProjects > PROJECT_LIMIT}
				<a
					href="/projects"
					class="inline-flex shrink-0 items-center gap-1 text-[12.5px] text-muted-foreground transition-colors hover:text-foreground">
					{t('common.home.view_all_projects')}
					<ArrowRightIcon class="size-3.5" />
				</a>
			{/if}
		</div>
	</div>

	{#if loading}
		<div
			class="cards"
			role="status"
			aria-busy="true"
			aria-label={t('common.home.loading_projects')}>
			{#each { length: 3 } as _, index (index)}
				<Skeleton class="h-56 rounded-2xl" />
			{/each}
		</div>
	{:else if loadError}
		<div class="rounded-2xl border border-destructive/30 p-6 text-sm text-destructive">
			{loadError}
		</div>
	{:else if overviews.length === 0}
		<div
			class="flex min-h-48 flex-col items-center justify-center gap-3 rounded-2xl border border-dashed p-8 text-center">
			<div class="bg-muted flex size-12 items-center justify-center rounded-full">
				<FolderKanbanIcon class="size-6" />
			</div>
			<h3 class="font-medium">{t('projects.projects.empty')}</h3>
			<p class="max-w-sm text-sm text-muted-foreground">
				{t('common.home.empty_projects_hint')}
			</p>
			{#if allowed('project.create')}
				<Button href="/projects">
					<PlusIcon class="size-4" />
					{t('projects.projects.create')}
				</Button>
			{/if}
		</div>
	{:else if visible.length === 0}
		<p class="py-4 text-sm text-muted-foreground">{t('projects.projects.no_matches')}</p>
	{:else}
		<div class="cards">
			{#each visible as overview (overview.project.id)}
				<OverviewProjectCard {overview} bind:failedApplicationIds />
			{/each}
		</div>
	{/if}
</section>

<style>
	.cards {
		display: grid;
		grid-template-columns: repeat(auto-fill, minmax(min(320px, 100%), 1fr));
		gap: 16px;
	}
</style>
