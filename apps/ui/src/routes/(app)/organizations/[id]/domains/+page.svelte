<script lang="ts">
	import { page } from '$app/state';
	import LockIcon from '@lucide/svelte/icons/lock';
	import LockOpenIcon from '@lucide/svelte/icons/lock-open';
	import SearchIcon from '@lucide/svelte/icons/search';
	import { Input } from '#lib/components/ui/input/index.js';
	import SkeletonRows from '#lib/components/custom/skeleton-rows.svelte';
	import { loadOrganizationApps, type OrganizationApp } from '#lib/utils/organization-apps.js';
	import { toneClass } from '#lib/utils/status-tone.js';
	import { t } from '#lib/i18n/index.svelte.js';

	const organizationId = $derived(page.params.id ?? '');

	let apps = $state<OrganizationApp[]>([]);
	let loading = $state(true);
	let error = $state('');
	let query = $state('');

	let rows = $derived(
		apps
			.flatMap((app) => app.domains.map((domain) => ({ app, domain })))
			.filter(({ app, domain }) => {
				const needle = query.trim().toLowerCase();
				return (
					!needle ||
					domain.host.toLowerCase().includes(needle) ||
					app.application.name.toLowerCase().includes(needle)
				);
			})
			.toSorted((a, b) => a.domain.host.localeCompare(b.domain.host)),
	);

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
		<h1 class="text-xl font-semibold tracking-tight">{t('common.org_apps.domains_title')}</h1>
		<p class="text-sm text-muted-foreground">{t('common.org_apps.domains_description')}</p>
	</div>

	<label class="lp-well relative flex max-w-sm items-center rounded-full bg-muted/60">
		<SearchIcon class="pointer-events-none absolute left-3 size-4 text-muted-foreground" />
		<Input
			bind:value={query}
			placeholder={t('common.org_apps.search')}
			aria-label={t('common.org_apps.search')}
			class="h-9 rounded-full border-0 bg-transparent pl-9 shadow-none" />
	</label>

	<section class="lp-raised overflow-hidden rounded-2xl border bg-card">
		{#if loading}
			<div class="p-4">
				<SkeletonRows rows={5} height="h-10" label={t('common.org_apps.loading')} />
			</div>
		{:else if error}
			<p class="p-6 text-sm text-destructive">{error}</p>
		{:else if rows.length === 0}
			<p class="p-6 text-sm text-muted-foreground">{t('common.org_apps.no_domains')}</p>
		{:else}
			<div class="domain-row head lp-groove-b text-xs text-muted-foreground">
				<span>{t('common.org_apps.col_domain')}</span>
				<span>{t('common.org_apps.col_app')}</span>
				<span class="col-hide">{t('common.org_apps.col_target')}</span>
				<span class="col-hide">{t('common.org_apps.col_tls')}</span>
			</div>
			{#each rows as { app, domain } (domain.id)}
				<a href="{app.href}?tab=domains" class="domain-row">
					<span class="flex min-w-0 items-center gap-2">
						{#if domain.https}
							<LockIcon class="size-3.5 shrink-0 text-success" />
						{:else}
							<LockOpenIcon class="size-3.5 shrink-0 text-muted-foreground" />
						{/if}
						<span class="truncate font-mono text-[13px]">{domain.host}</span>
						{#if domain.is_primary}
							<span class="shrink-0 rounded-full px-1.5 py-px text-[10.5px] {toneClass('info')}"
								>{t('common.org_apps.primary')}</span>
						{/if}
					</span>
					<span class="min-w-0">
						<span class="block truncate">{app.application.name}</span>
						<small class="block truncate text-xs text-muted-foreground"
							>{app.project.name} / {app.environment.name}</small>
					</span>
					<span class="col-hide font-mono text-xs text-muted-foreground">:{domain.port}</span>
					<span class="col-hide text-xs text-muted-foreground">
						{domain.https
							? (domain.certificate_name ?? t('common.org_apps.https'))
							: t('common.org_apps.http')}
					</span>
				</a>
			{/each}
		{/if}
	</section>
</div>

<style>
	.domain-row {
		display: grid;
		grid-template-columns: minmax(0, 1.5fr) minmax(0, 1fr) 80px minmax(0, 0.7fr);
		gap: 14px;
		align-items: center;
		padding: 10px 16px;
	}
	a.domain-row {
		transition: background-color 0.15s var(--lp-ease);
	}
	a.domain-row:hover {
		background: var(--muted);
	}
	a.domain-row + a.domain-row {
		border-top: 1px solid var(--lp-emboss);
	}
	@media (max-width: 640px) {
		.domain-row {
			grid-template-columns: minmax(0, 1fr) minmax(0, 0.8fr);
		}
		.col-hide {
			display: none;
		}
	}
</style>
