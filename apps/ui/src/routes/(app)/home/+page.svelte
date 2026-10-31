<script lang="ts">
	import { brandedTitle } from '#lib/ee/branding/branding.svelte.js';
	import { onMount } from 'svelte';
	import BoxIcon from '@lucide/svelte/icons/box';
	import GlobeIcon from '@lucide/svelte/icons/globe';
	import CalendarClockIcon from '@lucide/svelte/icons/calendar-clock';
	import CalendarPlusIcon from '@lucide/svelte/icons/calendar-plus';
	import PlusIcon from '@lucide/svelte/icons/plus';
	import ArrowRightIcon from '@lucide/svelte/icons/arrow-right';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import SkeletonRows from '#lib/components/custom/skeleton-rows.svelte';
	import { getMe } from '#lib/clients/auth-client.js';
	import { listOrganizations, type OrganizationDto } from '#lib/clients/organization-client.js';
	import { listScheduledTasks, type ScheduledTask } from '#lib/clients/scheduled-tasks-client.js';
	import { resolveSelectedOrganization } from '#lib/utils/selected-organization.js';
	import {
		filterApps,
		loadOrganizationApps,
		statusTone,
		type OrganizationApp,
	} from '#lib/utils/organization-apps.js';
	import { toneDot, type StatusTone } from '#lib/utils/status-tone.js';
	import { describeCron } from '#lib/utils/cron.js';
	import { relativeTime } from '#lib/utils/relative-time.js';
	import { toast } from '#lib/utils/toast.js';
	import { allowed } from '#lib/state/permissions.svelte.js';
	import KpiCard from '#lib/components/home/kpi-card.svelte';
	import ContainersKpi from '#lib/components/home/containers-kpi.svelte';
	import AppsTable from '#lib/components/home/apps-table.svelte';
	import OverviewSection from '#lib/components/home/overview-section.svelte';
	import SystemStatusSection from '#lib/components/home/system-status-section.svelte';
	import ActivitySection from '#lib/components/home/activity-section.svelte';
	import FullOnboardingChecklist from '#lib/components/onboarding/full-checklist.svelte';
	import { onboardingState } from '#lib/state/onboarding.svelte.js';
	import { i18n, t } from '#lib/i18n/index.svelte.js';

	const LATEST_LIMIT = 7;
	const UP_NEXT_LIMIT = 4;

	let organization = $state<OrganizationDto | null>(null);
	let userName = $state('');
	let isSysadmin = $state(false);
	let loading = $state(true);
	let loadError = $state('');

	let apps = $state<OrganizationApp[]>([]);
	let appsLoading = $state(true);
	let appsError = $state('');
	let tasks = $state<ScheduledTask[]>([]);
	let tasksLoading = $state(true);
	let tasksError = $state('');
	let failedApplicationIds = $state(new Set<string>());

	let orgBase = $derived(
		organization ? `/organizations/${encodeURIComponent(organization.id)}` : '',
	);
	let latest = $derived(filterApps(apps, '').slice(0, LATEST_LIMIT));
	let projectCount = $derived(new Set(apps.map((app) => app.project.id)).size);
	let toneCounts = $derived(
		apps.reduce(
			(counts, app) => {
				counts[statusTone(app.status)]++;
				return counts;
			},
			{ ok: 0, warn: 0, danger: 0, info: 0, neutral: 0 } as Record<StatusTone, number>,
		),
	);
	let domainCount = $derived(apps.reduce((sum, app) => sum + app.domains.length, 0));
	let httpsCount = $derived(
		apps.reduce((sum, app) => sum + app.domains.filter((domain) => domain.https).length, 0),
	);
	let enabledTasks = $derived(tasks.filter((task) => task.enabled));
	let upNext = $derived(
		enabledTasks
			.filter((task) => task.next_run_at !== null)
			.toSorted((a, b) => a.next_run_at! - b.next_run_at!)
			.slice(0, UP_NEXT_LIMIT),
	);

	onMount(() => void loadOrganization());

	async function loadOrganization() {
		loading = true;
		loadError = '';
		try {
			const [me, organizations] = await Promise.all([getMe(), listOrganizations()]);
			isSysadmin = me.is_sysadmin;
			userName = me.name;
			organization = resolveSelectedOrganization(organizations);
			if (organization) loadDashboard(organization.id);
		} catch (error) {
			loadError = error instanceof Error ? error.message : t('projects.projects.no_organization');
			toast.error(loadError);
		} finally {
			loading = false;
		}
	}

	function message(error: unknown, fallback: string) {
		return error instanceof Error ? error.message : fallback;
	}

	function loadDashboard(organizationId: string) {
		appsLoading = tasksLoading = true;
		loadOrganizationApps(organizationId)
			.then((next) => (apps = next))
			.catch((error) => (appsError = message(error, t('common.org_apps.load_failed'))))
			.finally(() => (appsLoading = false));
		listScheduledTasks(organizationId)
			.then((next) => (tasks = next))
			.catch((error) => (tasksError = message(error, t('organizations.tasks.load_failed'))))
			.finally(() => (tasksLoading = false));
	}
</script>

<svelte:head>
	<title>{brandedTitle(t('common.home.home'))}</title>
</svelte:head>

{#snippet legend(tone: StatusTone, label: string, square = false)}
	<span class="inline-flex items-center gap-1.5">
		<i class="{square ? 'size-2' : 'h-2.5 w-[3px]'} rounded-[2px] {toneDot(tone)}"></i>{label}
	</span>
{/snippet}

<div class="flex flex-col gap-6 px-4 py-6 sm:px-6 sm:py-8">
	<div class="flex flex-wrap items-end justify-between gap-4">
		<div class="min-w-0">
			<h1
				class="text-2xl leading-tight font-semibold tracking-[-.035em] break-words sm:text-[30px]">
				{userName ? t('common.home.welcome_back', { name: userName }) : t('common.home.home')}
			</h1>
			<p class="mt-1.5 text-sm text-muted-foreground">
				{organization ? t('common.home.overview') : t('common.home.select_organization')}
			</p>
		</div>
		{#if organization}
			<div class="flex flex-wrap items-center gap-2 max-sm:w-full">
				{#if allowed('scheduled_task.manage')}
					<Button variant="outline" href="{orgBase}/scheduled-tasks" class="max-sm:flex-1">
						<CalendarPlusIcon class="size-4" />
						{t('organizations.task_ui.new_task')}
					</Button>
				{/if}
				{#if allowed('project.create')}
					<Button href="/projects" class="max-sm:flex-1">
						<PlusIcon class="size-4" />
						{t('projects.projects.create')}
					</Button>
				{/if}
			</div>
		{/if}
	</div>

	{#if loading}
		<div
			class="kpis"
			role="status"
			aria-busy="true"
			aria-label={t('common.home.dashboard_loading')}>
			{#each { length: 4 } as _, index (index)}
				<Skeleton class="h-36 rounded-2xl" />
			{/each}
		</div>
		<div class="columns">
			<Skeleton class="h-96 rounded-2xl" />
			<Skeleton class="h-64 rounded-2xl" />
		</div>
	{:else if loadError}
		<div class="rounded-2xl border border-destructive/30 bg-card p-6 text-sm text-destructive">
			{loadError}
		</div>
	{:else if !organization}
		<div
			class="flex flex-col items-start gap-3 rounded-2xl border border-dashed border-foreground/15 bg-card p-8">
			<h2 class="font-medium">{t('common.home.no_organization')}</h2>
			<p class="text-sm text-muted-foreground">
				{t('common.home.no_organization_hint')}
			</p>
			<Button variant="outline" href="/organizations">{t('common.home.view_organizations')}</Button>
		</div>
	{:else}
		{#if onboardingState.summary && !onboardingState.summary.dismissed}
			<FullOnboardingChecklist summary={onboardingState.summary} />
		{/if}

		<div class="kpis">
			{#if appsLoading}
				<Skeleton class="min-h-36 rounded-2xl" />
			{:else}
				<KpiCard
					title={t('common.org_apps.kpi_apps')}
					sub={t('common.home.apps_across', { count: apps.length, projects: projectCount })}
					value={String(toneCounts.ok)}
					hint={t('common.home.running')}
					href="{orgBase}/apps"
					icon={BoxIcon}
					ticks={apps.map((app) => statusTone(app.status))}>
					<p class="flex flex-wrap gap-x-3.5 gap-y-1 text-xs text-muted-foreground">
						{@render legend('ok', t('common.org_apps.running_count', { count: toneCounts.ok }))}
						{#if toneCounts.danger}
							{@render legend(
								'danger',
								t('common.home.failed_count', { count: toneCounts.danger }),
							)}
						{/if}
						{@render legend(
							'neutral',
							t('common.home.other_count', {
								count: toneCounts.warn + toneCounts.info + toneCounts.neutral,
							}),
						)}
					</p>
				</KpiCard>
			{/if}
			{#if allowed('server.read')}
				<ContainersKpi
					organizationId={organization.id}
					href={isSysadmin
						? '/system?tab=containers'
						: `/organizations/${organization.id}/servers`} />
			{/if}
			{#if appsLoading}
				<Skeleton class="min-h-36 rounded-2xl" />
			{:else}
				<KpiCard
					title={t('common.org_apps.kpi_domains')}
					value={String(domainCount)}
					href="{orgBase}/domains"
					icon={GlobeIcon}
					tickShape="square"
					ticks={[
						...Array<StatusTone>(httpsCount).fill('ok'),
						...Array<StatusTone>(domainCount - httpsCount).fill('neutral'),
					]}>
					<p class="flex flex-wrap gap-x-3.5 gap-y-1 text-xs text-muted-foreground">
						{@render legend('ok', t('common.home.domains_https', { count: httpsCount }), true)}
						{@render legend(
							'neutral',
							t('common.home.domains_http', { count: domainCount - httpsCount }),
							true,
						)}
					</p>
				</KpiCard>
			{/if}
			{#if tasksLoading}
				<Skeleton class="min-h-36 rounded-2xl" />
			{:else}
				<KpiCard
					title={t('common.org_apps.kpi_tasks')}
					sub={tasksError
						? ''
						: t('common.home.tasks_sub', {
								active: enabledTasks.length,
								paused: tasks.length - enabledTasks.length,
							})}
					value={tasksError ? '-' : String(tasks.length)}
					href="{orgBase}/scheduled-tasks"
					icon={CalendarClockIcon}
					ticks={tasks.map((task) => (task.enabled ? 'ok' : 'neutral'))} />
			{/if}
		</div>

		<div class="columns">
			<div class="flex min-w-0 flex-col gap-6">
				<OverviewSection organizationId={organization.id} />

				<section class="lp-raised min-w-0 rounded-2xl border bg-card">
					<header class="lp-groove-b flex items-center justify-between gap-3 px-4 py-3">
						<h2 class="text-[15px] font-semibold tracking-tight">
							{t('common.org_apps.apps_title')}
						</h2>
						{#if !appsLoading && apps.length}
							<a
								href="{orgBase}/apps"
								class="inline-flex items-center gap-1 text-[12.5px] text-muted-foreground transition-colors hover:text-foreground">
								{t('common.org_apps.view_all', { count: apps.length })}
								<ArrowRightIcon class="size-3.5" />
							</a>
						{/if}
					</header>
					{#if appsLoading}
						<div class="p-4">
							<SkeletonRows rows={LATEST_LIMIT} height="h-9" label={t('common.org_apps.loading')} />
						</div>
					{:else if appsError}
						<p class="p-6 text-sm text-destructive">{appsError}</p>
					{:else if latest.length === 0}
						<p class="p-6 text-sm text-muted-foreground">{t('common.home.no_applications')}</p>
					{:else}
						<AppsTable apps={latest} bind:failedApplicationIds />
					{/if}
				</section>
			</div>

			<aside class="flex min-w-0 flex-col gap-4">
				<ActivitySection organizationId={organization.id} limit={6} />

				<section class="lp-raised min-w-0 overflow-hidden rounded-2xl border bg-card">
					<header class="lp-groove-b flex items-center justify-between gap-3 px-4 py-3">
						<h2 class="text-[15px] font-semibold tracking-tight">{t('common.org_apps.up_next')}</h2>
						<a
							href="{orgBase}/scheduled-tasks"
							class="text-[12.5px] text-muted-foreground transition-colors hover:text-foreground"
							>{t('common.org_apps.kpi_tasks')}</a>
					</header>
					{#if tasksLoading}
						<div class="p-3">
							<SkeletonRows rows={UP_NEXT_LIMIT} height="h-10" />
						</div>
					{:else if tasksError}
						<p class="p-6 text-sm text-destructive">{tasksError}</p>
					{:else if upNext.length === 0}
						<p class="p-6 text-sm text-muted-foreground">{t('common.org_apps.no_tasks')}</p>
					{:else}
						<ol class="flex flex-col p-1.5">
							{#each upNext as task (task.id)}
								<li>
									<a
										href="{orgBase}/scheduled-tasks"
										class="grid grid-cols-[minmax(0,1fr)_auto] gap-2.5 rounded-lg px-2.5 py-2 transition-colors hover:bg-muted">
										<span class="min-w-0">
											<b class="block truncate font-mono text-[12.5px] font-medium">{task.name}</b>
											<small class="block truncate text-xs text-muted-foreground"
												>{describeCron(task.cron_expression) ?? task.cron_expression}</small>
										</span>
										<time
											class="text-xs whitespace-nowrap text-muted-foreground"
											datetime={new Date(task.next_run_at!).toISOString()}
											>{relativeTime(task.next_run_at!, Date.now(), i18n.locale)}</time>
									</a>
								</li>
							{/each}
						</ol>
					{/if}
				</section>
			</aside>
		</div>

		{#if isSysadmin}
			<SystemStatusSection />
		{/if}
	{/if}
</div>

<style>
	.kpis {
		display: grid;
		grid-template-columns: repeat(auto-fit, minmax(min(240px, 100%), 1fr));
		gap: 12px;
	}
	.columns {
		display: grid;
		grid-template-columns: minmax(0, 1fr) 340px;
		gap: 24px;
		align-items: start;
	}
	@media (max-width: 1100px) {
		.columns {
			grid-template-columns: minmax(0, 1fr);
		}
	}
	/* Phones: one swipeable row instead of a tall stack of cards. */
	@media (max-width: 640px) {
		.kpis {
			display: flex;
			overflow-x: auto;
			scroll-snap-type: x mandatory;
			scrollbar-width: none;
			margin-inline: -1rem;
			padding: 2px 1rem 6px;
			scroll-padding-inline: 1rem;
		}
		.kpis > :global(*) {
			flex: 0 0 78%;
			scroll-snap-align: start;
		}
	}
</style>
