<script lang="ts">
	import type { EnvironmentDto, ProjectDto } from '#lib/clients/project-client.js';
	import PencilIcon from '@lucide/svelte/icons/pencil';
	import Trash2Icon from '@lucide/svelte/icons/trash-2';
	import ShieldCheckIcon from '@lucide/svelte/icons/shield-check';
	import FlaskConicalIcon from '@lucide/svelte/icons/flask-conical';
	import CellChart from '#lib/components/custom/cell-chart.svelte';
	import ChartTooltip from '#lib/components/custom/chart-tooltip.svelte';
	import ApplicationIcon from '#lib/components/projects/application-icon.svelte';
	import {
		compactNumber,
		sparklineTime,
		sumSparklines,
		type Sparklines,
	} from '#lib/clients/analytics-client.js';
	import { isDisplayableImageUrl } from '#lib/utils/application-card-icon.js';
	import {
		applicationHref,
		statusTone,
		type OrganizationApp,
	} from '#lib/utils/organization-apps.js';
	import { toneDot } from '#lib/utils/status-tone.js';
	import { i18n, t } from '#lib/i18n/index.svelte.js';
	import { cn } from '#lib/utils.js';

	const SHOWN = 6;
	const CHART_COLS = 48;
	let hovered = $state<number | null>(null);
	let chartHost = $state<HTMLDivElement | null>(null);

	let {
		project,
		environments = [],
		statuses = {},
		query = '',
		sparklines,
		onedit,
		ondelete,
	}: {
		project: ProjectDto;
		environments?: EnvironmentDto[];
		/** Organization request sparklines; omit to hide the chart. */
		sparklines?: Sparklines;
		/** Deployment status per application id; missing while it loads. */
		statuses?: Record<string, OrganizationApp['status']>;
		/** Lower-cased filter; only matching apps are listed when set. */
		query?: string;
		/** Omit to hide the button (caller lacks the permission). */
		onedit?: (project: ProjectDto) => void;
		ondelete?: (project: ProjectDto) => void;
	} = $props();

	let iconFailed = $state(false);

	let apps = $derived(
		environments.flatMap((environment) =>
			environment.applications.map((application) => ({ application, environment })),
		),
	);
	let projectMatches = $derived(
		!query ||
			project.name.toLowerCase().includes(query) ||
			project.description.toLowerCase().includes(query),
	);
	let listed = $derived(
		projectMatches
			? apps
			: apps.filter(
					({ application, environment }) =>
						application.name.toLowerCase().includes(query) ||
						environment.name.toLowerCase().includes(query),
				),
	);
	let failedIconIds = $state(new Set<string>());
	let requests = $derived(
		sparklines &&
			sumSparklines(
				sparklines,
				apps.map(({ application }) => application.id),
			),
	);
	let bucketTime = $derived(sparklines && sparklineTime(sparklines, i18n.locale));
	/** Apps with traffic in the hovered bucket, busiest first. */
	let hoveredApps = $derived(
		hovered === null || !sparklines
			? []
			: apps
					.map(({ application }) => ({
						application,
						count: sparklines.applications[application.id]?.[hovered!] ?? 0,
					}))
					.filter(({ count }) => count > 0)
					.sort((a, b) => b.count - a.count),
	);
	let running = $derived(apps.filter(({ application }) => statuses[application.id] === 'running'));
</script>

<article
	class="pcard lp-raised group relative flex flex-col gap-3.5 rounded-2xl border bg-card p-[18px]">
	<a
		class="absolute inset-0 rounded-2xl outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2"
		href={`/projects/${project.id}`}
		aria-label={t('projects.projects.open_project', { name: project.name })}></a>

	<header class="flex items-start gap-3">
		<span
			class="tile flex size-9 shrink-0 items-center justify-center overflow-hidden rounded-[9px] border bg-muted text-sm font-semibold">
			{#if isDisplayableImageUrl(project.icon_url) && !iconFailed}
				<img
					src={project.icon_url}
					alt=""
					loading="lazy"
					class="size-full object-contain"
					onerror={() => (iconFailed = true)} />
			{:else}
				{project.name.charAt(0).toUpperCase()}
			{/if}
		</span>
		<div class="min-w-0 flex-1">
			<h2 class="truncate text-[15px] font-semibold tracking-tight">{project.name}</h2>
			<p class="truncate text-[12.5px] text-muted-foreground">
				{project.description || t('projects.projects.no_description')}
			</p>
		</div>
		<div
			class="relative z-10 -mt-1 -mr-1 flex opacity-100 transition-opacity sm:opacity-0 sm:group-focus-within:opacity-100 sm:group-hover:opacity-100">
			{#if onedit}
				<button
					type="button"
					class="icon-btn text-muted-foreground hover:text-foreground"
					aria-label={t('projects.projects.edit_project_named', { name: project.name })}
					onclick={() => onedit(project)}>
					<PencilIcon class="size-3.5" />
				</button>
			{/if}
			{#if ondelete}
				<button
					type="button"
					class="icon-btn text-muted-foreground hover:text-destructive"
					aria-label={t('projects.projects.delete_project_named', { name: project.name })}
					onclick={() => ondelete(project)}>
					<Trash2Icon class="size-3.5" />
				</button>
			{/if}
		</div>
		<span class="chip">{t('projects.projects.apps_count', { count: apps.length })}</span>
	</header>

	{#if requests && apps.length}
		<!-- relative: lift above the card's stretched link so the chart gets hover -->
		<div class="relative" bind:this={chartHost}>
			<CellChart
				values={requests}
				cols={CHART_COLS}
				rows={9}
				tone="muted"
				onhover={(i) => (hovered = i)}
				ariaLabel={t('projects.environment.requests_24h')} />
		</div>
		<ChartTooltip host={chartHost} column={hovered} cols={CHART_COLS}>
			{#if hovered !== null}
				<div class="flex items-center justify-between gap-3 font-mono text-[11.5px] text-muted-foreground">
					<span>{bucketTime?.(hovered)}</span>
					<b class="font-medium text-foreground tabular-nums"
						>{t('analytics.panel.requests_count', { count: requests[hovered] ?? 0 })}</b>
				</div>
				{#if hoveredApps.length}
					<ul class="mt-2 grid grid-cols-[auto_1fr_auto] items-center gap-x-2 gap-y-1.5 text-[12.5px]">
						{#each hoveredApps as { application, count } (application.id)}
							<li class="contents">
								<span class="flex size-5 items-center justify-center overflow-hidden rounded bg-muted">
									<ApplicationIcon
										templateKind={application.template_kind}
										catalogIconUrl={application.catalog_icon_url}
										iconUrl={application.icon_url}
										applicationId={application.id}
										class="size-3.5"
										bind:failedApplicationIds={failedIconIds} />
								</span>
								<span class="truncate">{application.name}</span>
								<span class="font-mono tabular-nums text-muted-foreground"
									>{compactNumber(count, i18n.locale)}</span>
							</li>
						{/each}
					</ul>
				{/if}
			{/if}
		</ChartTooltip>
		<div
			class="mt-2 flex items-center justify-between font-mono text-[11.5px] text-muted-foreground">
			<span>{t('projects.environment.requests_24h')}</span>
			<span class="tabular-nums"
				>{compactNumber(
					requests.reduce((a, b) => a + b, 0),
					i18n.locale,
				)}</span>
		</div>
	{/if}

	{#if listed.length}
		<div class="flex flex-wrap gap-1.5">
			{#each listed.slice(0, SHOWN) as { application, environment } (application.id)}
				{@const status = statuses[application.id]}
				<a
					href={applicationHref(project.id, environment.id, application.id)}
					class="chip app-chip relative z-10">
					{#if status !== undefined}
						<span class={cn('size-1.5 shrink-0 rounded-full', toneDot(statusTone(status)))}></span>
					{/if}
					{application.name}
				</a>
			{/each}
			{#if listed.length > SHOWN}
				<span class="chip"
					>{t('projects.projects.more_apps', { count: listed.length - SHOWN })}</span>
			{/if}
		</div>
	{/if}

	<footer class="lp-groove-t mt-auto flex flex-wrap gap-1.5 pt-3.5">
		{#each environments as environment (environment.id)}
			<span class="chip">
				{#if environment.is_production}<ShieldCheckIcon class="size-3" />{:else}<FlaskConicalIcon
						class="size-3" />{/if}
				{environment.name}
			</span>
		{/each}
		{#if apps.length}
			<span class="chip ms-auto"
				>{t('projects.projects.running_of', {
					running: running.length,
					total: apps.length,
				})}</span>
		{/if}
	</footer>
</article>

<style>
	.pcard {
		transition:
			transform 0.35s var(--lp-ease),
			border-color 0.25s var(--lp-ease),
			box-shadow 0.25s var(--lp-ease);
	}
	.pcard:hover {
		transform: translateY(-3px);
		border-color: color-mix(in srgb, var(--primary) 35%, var(--border));
	}
	.tile {
		box-shadow:
			inset 1px 1px 0 var(--lp-bevel),
			inset -1px -1px 0 var(--lp-emboss);
	}
	.chip {
		display: inline-flex;
		align-items: center;
		gap: 6px;
		height: 24px;
		padding: 0 8px;
		border-radius: 6px;
		background: var(--muted);
		box-shadow:
			inset 1px 1px 0 var(--lp-bevel),
			inset -1px -1px 0 var(--lp-emboss);
		color: var(--muted-foreground);
		font-size: 12px;
		white-space: nowrap;
	}
	.app-chip {
		font-family: var(--font-mono);
		font-size: 11.5px;
		transition: color 0.15s;
	}
	.app-chip:hover {
		color: var(--foreground);
	}
	.icon-btn {
		display: inline-flex;
		align-items: center;
		justify-content: center;
		width: 28px;
		height: 28px;
		border-radius: 8px;
		transition:
			background-color 0.15s,
			color 0.15s;
	}
	.icon-btn:hover {
		background: var(--muted);
	}
	.icon-btn:focus-visible {
		outline: none;
		box-shadow: 0 0 0 3px color-mix(in srgb, var(--ring) 30%, transparent);
	}
	@media (prefers-reduced-motion: reduce) {
		.pcard,
		.pcard:hover {
			transition: none;
			transform: none;
		}
	}
</style>
