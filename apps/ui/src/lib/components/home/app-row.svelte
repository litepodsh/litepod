<script lang="ts">
	import ApplicationIcon from '#lib/components/projects/application-icon.svelte';
	import { primaryDomain, statusTone, type OrganizationApp } from '#lib/utils/organization-apps.js';
	import { toneClass } from '#lib/utils/status-tone.js';
	import { t } from '#lib/i18n/index.svelte.js';

	let {
		app,
		failedApplicationIds = $bindable(new Set<string>()),
	}: { app: OrganizationApp; failedApplicationIds?: Set<string> } = $props();

	let domain = $derived(primaryDomain(app.domains));
	let statusLabel = $derived(
		app.status ? t(`common.org_apps.status_${app.status}`) : t('common.org_apps.status_unknown'),
	);
</script>

<a href={app.href} class="app-row" data-testid="app-row">
	<span class="flex min-w-0 items-center gap-2.5">
		<span class="grid size-8 shrink-0 place-items-center rounded-lg bg-muted">
			<ApplicationIcon
				templateKind={app.application.template_kind}
				catalogIconUrl={app.application.catalog_icon_url}
				iconUrl={app.application.icon_url}
				applicationId={app.application.id}
				bind:failedApplicationIds
				class="size-4" />
		</span>
		<span class="min-w-0">
			<span class="block truncate font-medium">{app.application.name}</span>
			<small class="block truncate text-xs text-muted-foreground"
				>{app.project.name} / {app.environment.name}</small>
		</span>
	</span>
	<span class="col-domain truncate font-mono text-xs text-muted-foreground"
		>{domain?.host ?? ''}{#if app.domains.length > 1}<span class="ms-1 opacity-70"
				>+{app.domains.length - 1}</span
			>{/if}</span>
	<span class="justify-self-end">
		<span class="rounded-full px-2 py-0.5 text-xs font-medium {toneClass(statusTone(app.status))}"
			>{statusLabel}</span>
	</span>
</a>

<style>
	.app-row {
		display: grid;
		/* Fixed status column: each row is its own grid, so `auto` would let the
		   domain column drift with the badge's width. */
		grid-template-columns: minmax(0, 1.3fr) minmax(0, 1fr) 6.5rem;
		gap: 14px;
		align-items: center;
		padding: 10px 16px;
		transition: background-color 0.15s var(--lp-ease);
	}
	.app-row:hover {
		background: var(--muted);
	}
	.app-row + :global(.app-row) {
		border-top: 1px solid var(--lp-emboss);
	}
	@media (max-width: 640px) {
		.app-row {
			grid-template-columns: minmax(0, 1fr) 6.5rem;
		}
		.col-domain {
			display: none;
		}
	}
</style>
