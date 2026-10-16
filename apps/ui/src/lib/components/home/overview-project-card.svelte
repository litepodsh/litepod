<script lang="ts">
	import FolderKanbanIcon from '@lucide/svelte/icons/folder-kanban';
	import ShieldCheckIcon from '@lucide/svelte/icons/shield-check';
	import FlaskConicalIcon from '@lucide/svelte/icons/flask-conical';
	import type {
		ApplicationDto,
		ApplicationStatsDto,
		ProjectDto,
	} from '#lib/clients/project-client.js';
	import { environmentApplicationsHref } from '#lib/utils/environment-route.js';
	import ApplicationIcon from '#lib/components/projects/application-icon.svelte';
	import { isDisplayableImageUrl } from '#lib/utils/application-card-icon.js';
	import { statusTone, type OrganizationApp } from '#lib/utils/organization-apps.js';
	import { toneDot } from '#lib/utils/status-tone.js';
	import { t } from '#lib/i18n/index.svelte.js';

	const CHIP_LIMIT = 6;

	let {
		overview,
		failedApplicationIds = $bindable(),
		class: className = '',
	}: {
		overview: {
			project: ProjectDto;
			environmentName: string;
			isProduction?: boolean;
			apps: {
				application: ApplicationDto;
				deployment: { status: string } | null;
				stats: ApplicationStatsDto | null;
			}[];
		};
		failedApplicationIds: Set<string>;
		class?: string;
	} = $props();

	let appsHref = $derived(
		environmentApplicationsHref(overview.project.id, overview.environmentName),
	);
	function status(app: (typeof overview.apps)[number]) {
		return (app.stats?.status ?? app.deployment?.status ?? null) as OrganizationApp['status'];
	}
	let running = $derived(overview.apps.filter((app) => status(app) === 'running').length);
	let live = $derived(overview.apps.filter((app) => app.stats?.cpu_percent != null));
	let cpu = $derived(live.reduce((sum, app) => sum + (app.stats!.cpu_percent ?? 0), 0));
	let memory = $derived(live.reduce((sum, app) => sum + (app.stats!.mem_usage_bytes ?? 0), 0));

	function formatBytes(bytes: number): string {
		const units = ['B', 'KB', 'MB', 'GB', 'TB'];
		let value = bytes;
		let unitIndex = 0;
		while (value >= 1024 && unitIndex < units.length - 1) {
			value /= 1024;
			unitIndex++;
		}
		return `${value.toFixed(unitIndex === 0 || value >= 10 ? 0 : 1)} ${units[unitIndex]}`;
	}
</script>

<article
	class="lp-raised relative flex h-full flex-col gap-3.5 rounded-2xl border bg-card p-[18px] transition-[border-color,translate] duration-300 hover:-translate-y-0.5 hover:border-primary/35 motion-reduce:transition-none motion-reduce:hover:translate-y-0 {className}">
	<header class="flex items-start gap-3">
		<span
			class="lp-inset grid size-9 shrink-0 place-items-center overflow-hidden rounded-[9px] border bg-muted">
			{#if isDisplayableImageUrl(overview.project.icon_url)}
				<img src={overview.project.icon_url} alt="" class="size-5 object-contain" />
			{:else}
				<FolderKanbanIcon class="size-4 text-muted-foreground" />
			{/if}
		</span>
		<span class="min-w-0 flex-1">
			<!-- The title link covers the card; the app chips sit above it. -->
			<a
				href={`/projects/${overview.project.id}`}
				class="block truncate text-[14.5px] font-semibold tracking-tight after:absolute after:inset-0 after:rounded-2xl after:content-['']"
				>{overview.project.name}</a>
			{#if overview.project.description}
				<span class="block truncate text-[12.5px] text-muted-foreground"
					>{overview.project.description}</span>
			{/if}
		</span>
		<span
			class="lp-raised inline-flex h-6 shrink-0 items-center rounded-md bg-muted px-2 text-xs text-muted-foreground"
			>{t('common.home.apps_count', { count: overview.apps.length })}</span>
	</header>

	{#if overview.apps.length === 0}
		<p class="text-sm text-muted-foreground">
			{t('common.home.no_applications')}
			<a class="relative z-10 text-primary underline-offset-4 hover:underline" href={appsHref}>
				{t('common.home.add_application')}
			</a>
		</p>
	{:else}
		<div class="flex flex-wrap gap-1.5">
			{#each overview.apps.slice(0, CHIP_LIMIT) as app (app.application.id)}
				<a
					href={`${appsHref}/${app.application.id}`}
					class="lp-raised relative z-10 inline-flex h-6 max-w-full items-center gap-1.5 rounded-md bg-muted px-2 font-mono text-[11.5px] text-muted-foreground transition-colors hover:text-foreground">
					<ApplicationIcon
						templateKind={app.application.template_kind}
						catalogIconUrl={app.application.catalog_icon_url}
						iconUrl={app.application.icon_url}
						applicationId={app.application.id}
						bind:failedApplicationIds
						class="size-3.5 shrink-0 rounded-sm" />
					<span class="truncate">{app.application.name}</span>
					<i class="size-1.5 shrink-0 rounded-full {toneDot(statusTone(status(app)))}"></i>
				</a>
			{/each}
			{#if overview.apps.length > CHIP_LIMIT}
				<span
					class="lp-raised inline-flex h-6 items-center rounded-md bg-muted px-2 text-xs text-muted-foreground"
					>{t('common.home.more_count', { count: overview.apps.length - CHIP_LIMIT })}</span>
			{/if}
		</div>
		{#if live.length}
			<p class="font-mono text-xs text-muted-foreground tabular-nums">
				{cpu.toFixed(0)}% CPU · {formatBytes(memory)}
			</p>
		{/if}
	{/if}

	<footer class="lp-groove-t mt-auto flex flex-wrap gap-1.5 pt-3.5">
		{#if overview.environmentName}
			<span
				class="lp-raised inline-flex h-6 items-center gap-1.5 rounded-md bg-muted px-2 text-xs text-muted-foreground">
				{#if overview.isProduction}
					<ShieldCheckIcon class="size-3.5" />
				{:else}
					<FlaskConicalIcon class="size-3.5" />
				{/if}
				{overview.environmentName}
			</span>
		{/if}
		<span
			class="lp-raised ms-auto inline-flex h-6 items-center rounded-md bg-muted px-2 text-xs text-muted-foreground"
			>{t('common.home.running_of', { running, total: overview.apps.length })}</span>
	</footer>
</article>
