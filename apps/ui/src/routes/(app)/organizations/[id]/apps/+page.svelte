<script lang="ts">
	import { page } from '$app/state';
	import SearchIcon from '@lucide/svelte/icons/search';
	import { Input } from '#lib/components/ui/input/index.js';
	import SearchableDropdown from '#lib/components/custom/searchable-dropdown.svelte';
	import SkeletonRows from '#lib/components/custom/skeleton-rows.svelte';
	import AppRow from '#lib/components/home/app-row.svelte';
	import {
		filterApps,
		loadOrganizationApps,
		type OrganizationApp,
	} from '#lib/utils/organization-apps.js';
	import { t } from '#lib/i18n/index.svelte.js';

	const organizationId = $derived(page.params.id ?? '');
	const ALL = '';

	let apps = $state<OrganizationApp[]>([]);
	let loading = $state(true);
	let error = $state('');
	let query = $state('');
	let projectId = $state(ALL);
	let failedApplicationIds = $state(new Set<string>());

	let projects = $derived(
		[...new Map(apps.map((app) => [app.project.id, app.project.name])).entries()].map(
			([value, label]) => ({ value, label }),
		),
	);
	let visible = $derived(filterApps(apps, query, projectId || null));

	$effect(() => {
		const id = organizationId;
		loading = true;
		error = '';
		loadOrganizationApps(id)
			.then((next) => (apps = next))
			.catch(
				(caught) =>
					(error = caught instanceof Error ? caught.message : t('common.org_apps.load_failed')),
			)
			.finally(() => (loading = false));
	});
</script>

<div class="flex flex-col gap-5 px-4 py-6 sm:px-6">
	<div>
		<h1 class="text-xl font-semibold tracking-tight">{t('common.org_apps.apps_title')}</h1>
		<p class="text-sm text-muted-foreground">{t('common.org_apps.apps_description')}</p>
	</div>

	<div class="flex flex-wrap items-center gap-2">
		<label
			class="lp-well relative flex min-w-0 flex-1 items-center rounded-full bg-muted/60 sm:max-w-sm">
			<SearchIcon class="pointer-events-none absolute left-3 size-4 text-muted-foreground" />
			<Input
				bind:value={query}
				placeholder={t('common.org_apps.search')}
				aria-label={t('common.org_apps.search')}
				class="h-9 rounded-full border-0 bg-transparent pl-9 shadow-none" />
		</label>
		<SearchableDropdown
			items={[{ value: ALL, label: t('common.org_apps.all_projects') }, ...projects]}
			bind:selectedValue={projectId}
			label={t('common.org_apps.all_projects')}
			searchPlaceholder={t('projects.common.search_projects')}
			emptyText={t('projects.common.no_project_found')}
			size="sm" />
	</div>

	<section class="lp-raised overflow-hidden rounded-2xl border bg-card">
		{#if loading}
			<div class="p-4">
				<SkeletonRows rows={5} height="h-10" label={t('common.org_apps.loading')} />
			</div>
		{:else if error}
			<p class="p-6 text-sm text-destructive">{error}</p>
		{:else if visible.length === 0}
			<p class="p-6 text-sm text-muted-foreground">{t('common.org_apps.empty')}</p>
		{:else}
			{#each visible as app (app.application.id)}
				<AppRow {app} bind:failedApplicationIds />
			{/each}
		{/if}
	</section>
</div>
