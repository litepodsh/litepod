<script lang="ts">
	import ExternalLinkIcon from '@lucide/svelte/icons/external-link';
	import ApplicationIcon from '#lib/components/projects/application-icon.svelte';
	import { primaryDomain, statusTone, type OrganizationApp } from '#lib/utils/organization-apps.js';
	import { toneClass, toneDot } from '#lib/utils/status-tone.js';
	import { relativeTime } from '#lib/utils/relative-time.js';
	import { i18n, t } from '#lib/i18n/index.svelte.js';

	/** The home "Apps" table: app, project, domains, containers, status and last deploy. */
	let {
		apps,
		failedApplicationIds = $bindable(new Set<string>()),
	}: {
		apps: OrganizationApp[];
		failedApplicationIds?: Set<string>;
	} = $props();

	function statusLabel(status: OrganizationApp['status']) {
		return status ? t(`common.org_apps.status_${status}`) : t('common.org_apps.status_unknown');
	}

	// API timestamps are epoch ms; tolerate seconds just in case.
	const ms = (value: number) => (value < 1e12 ? value * 1000 : value);
</script>

<div class="apps-table" role="table" aria-label={t('common.org_apps.apps_title')}>
	<div class="row head lp-groove-b" role="row">
		<span role="columnheader">{t('common.home.col_app')}</span>
		<span role="columnheader" class="c-project">{t('common.home.col_project')}</span>
		<span role="columnheader" class="c-domain">{t('common.org_apps.col_domain')}</span>
		<span role="columnheader" class="c-ctr">{t('common.home.containers')}</span>
		<span role="columnheader">{t('common.home.col_status')}</span>
		<span role="columnheader" class="c-age text-right">{t('common.home.col_deployed')}</span>
	</div>
	{#each apps as app (app.application.id)}
		{@const domain = primaryDomain(app.domains)}
		{@const count = app.containers}
		<div class="row body" role="row" data-testid="apps-table-row">
			<span role="cell" class="flex min-w-0 items-center gap-2.5">
				<span class="lp-inset grid size-7 shrink-0 place-items-center rounded-lg bg-muted">
					<ApplicationIcon
						templateKind={app.application.template_kind}
						catalogIconUrl={app.application.catalog_icon_url}
						iconUrl={app.application.icon_url}
						applicationId={app.application.id}
						bind:failedApplicationIds
						class="size-4" />
				</span>
				<span class="min-w-0">
					<!-- The name link covers the whole row; the domain link sits above it. -->
					<a
						href={app.href}
						class="block truncate font-medium after:absolute after:inset-0 after:content-['']"
						>{app.application.name}</a>
					<small class="block truncate font-mono text-[11px] text-muted-foreground"
						><span class="s-project">{`${app.project.name} / `}</span>{app.environment.name}</small>
				</span>
			</span>
			<span role="cell" class="c-project truncate text-[12.5px] text-muted-foreground"
				>{app.project.name}</span>
			<span role="cell" class="c-domain flex min-w-0 items-center gap-1.5 text-[12.5px]">
				{#if domain}
					<a
						href={domain.url}
						target="_blank"
						rel="noreferrer"
						class="relative z-10 inline-flex min-w-0 items-center gap-1 text-muted-foreground transition-colors hover:text-primary">
						<span class="truncate">{domain.host}</span>
						<ExternalLinkIcon class="size-3 shrink-0" />
					</a>
					{#if app.domains.length > 1}
						<span class="group relative z-10 shrink-0">
							<button
								type="button"
								class="h-5 rounded-full bg-primary/10 px-1.5 text-[11.5px] font-medium text-primary"
								aria-label={t('common.home.show_all_domains')}>+{app.domains.length - 1}</button>
							<span
								role="tooltip"
								class="lp-raised pointer-events-none absolute top-[calc(100%+6px)] left-0 z-20 flex min-w-64 flex-col rounded-xl border bg-popover p-1.5 opacity-0 transition-opacity group-focus-within:pointer-events-auto group-focus-within:opacity-100 group-hover:pointer-events-auto group-hover:opacity-100">
								{#each app.domains as item (item.id)}
									<a
										href={item.url}
										target="_blank"
										rel="noreferrer"
										class="grid grid-cols-[minmax(0,1fr)_auto] gap-3 rounded-md px-2 py-1.5 hover:bg-muted">
										<span class="truncate">{item.host}</span>
										<span class="font-mono text-[11.5px] text-muted-foreground">:{item.port}</span>
									</a>
								{/each}
							</span>
						</span>
					{/if}
				{:else}
					<span class="text-muted-foreground">-</span>
				{/if}
			</span>
			<span role="cell" class="c-ctr text-[12.5px] whitespace-nowrap text-muted-foreground">
				{#if count}
					<span class="inline-flex items-center gap-1.5">
						<i
							class="size-1.5 rounded-full {toneDot(
								count.total === 0 ? 'neutral' : count.running === count.total ? 'ok' : 'warn',
							)}"></i>
						<span class="font-mono tabular-nums">{count.running}/{count.total}</span>
					</span>
				{:else}-{/if}
			</span>
			<span role="cell">
				<span
					class="inline-flex items-center gap-1.5 rounded-full px-2 py-0.5 text-xs font-medium whitespace-nowrap {toneClass(
						statusTone(app.status),
					)}">
					<i class="size-1.5 rounded-full bg-current"></i>
					{statusLabel(app.status)}
				</span>
			</span>
			<span role="cell" class="c-age text-right text-xs whitespace-nowrap text-muted-foreground">
				{#if app.deployedAt}
					<time datetime={new Date(ms(app.deployedAt)).toISOString()}
						>{relativeTime(ms(app.deployedAt), Date.now(), i18n.locale)}</time>
				{/if}
			</span>
		</div>
	{/each}
</div>

<style>
	.apps-table {
		container-type: inline-size;
	}
	.row {
		display: grid;
		grid-template-columns: minmax(0, 1.4fr) minmax(0, 0.8fr) minmax(0, 1.6fr) 5.5rem 7rem minmax(
				4.5rem,
				auto
			);
		gap: 14px;
		align-items: center;
		padding: 11px 16px;
	}
	.head {
		padding-block: 10px;
		font-size: 12px;
		color: var(--muted-foreground);
	}
	.body {
		position: relative;
		transition: background-color 0.15s var(--lp-ease);
	}
	.body:hover {
		background: var(--muted);
	}
	/* The card is not overflow-hidden (the domains popover must escape it). */
	.body:last-child {
		border-radius: 0 0 calc(1rem - 1px) calc(1rem - 1px);
	}
	.body + .body {
		border-top: 1px solid var(--lp-emboss);
	}
	.s-project {
		display: none;
	}
	@container (max-width: 760px) {
		.row {
			grid-template-columns: minmax(0, 1.4fr) minmax(0, 1.4fr) 7rem;
		}
		.s-project {
			display: inline;
		}
		.c-project,
		.c-ctr,
		.c-age {
			display: none;
		}
	}
	@container (max-width: 480px) {
		.row {
			grid-template-columns: minmax(0, 1fr) auto;
		}
		.c-domain {
			display: none;
		}
	}
	@media (prefers-reduced-motion: reduce) {
		.body {
			transition: none;
		}
	}
</style>
