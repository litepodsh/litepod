<script lang="ts">
	import { onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import { getProject } from '#lib/clients/project-client.js';
	import { listOrganizations } from '#lib/clients/organization-client.js';
	import {
		resolveSelectedOrganization,
		setSelectedOrganizationId,
	} from '#lib/utils/selected-organization.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import { t } from '#lib/i18n/index.svelte.js';

	const projectId = page.params.project_id ?? '';
	let error = $state<string | null>(null);

	onMount(async () => {
		try {
			const organization = resolveSelectedOrganization(await listOrganizations());
			if (!organization) {
				error = t('projects.projects.choose_organization');
				return;
			}
			setSelectedOrganizationId(organization.id);
			const detail = await getProject(organization.id, projectId);
			const defaultEnvironment =
				detail.environments.find((environment) => environment.is_production) ??
				detail.environments[0];
			if (!defaultEnvironment) {
				error = t('projects.projects.no_environments');
				return;
			}
			await goto(
				`/projects/${encodeURIComponent(projectId)}/environments/${encodeURIComponent(defaultEnvironment.id)}`,
				{ replace: true },
			);
		} catch (caught) {
			error = caught instanceof Error ? caught.message : t('projects.projects.unable_to_load');
		}
	});
</script>

<div class="flex flex-col gap-5 px-4 py-6 sm:px-6">
	{#if error}
		<div class="lp-raised rounded-2xl border border-destructive/30 bg-card p-6">
			<p class="font-medium">{t('projects.projects.unable_to_load')}</p>
			<p class="mt-1 text-sm text-muted-foreground">{error}</p>
			<Button variant="outline" size="sm" class="mt-4" href="/projects"
				>{t('projects.projects.title')}</Button>
		</div>
	{:else}
		<!-- Same shape as the environment page it redirects to, so the swap does not jump. -->
		<div role="status" aria-busy="true" aria-label={t('projects.common.loading_project')}>
			<div class="flex items-center gap-3.5">
				<Skeleton class="size-12 rounded-xl" />
				<div class="flex flex-col gap-2">
					<Skeleton class="h-5 w-40" />
					<Skeleton class="h-4 w-64 max-w-[60vw]" />
				</div>
			</div>
			<Skeleton class="mt-5 h-16 rounded-2xl" />
			<div class="mt-5 grid grid-cols-1 gap-4 md:grid-cols-2 xl:grid-cols-3">
				{#each { length: 3 } as _, index (index)}
					<Skeleton class="h-48 rounded-2xl" />
				{/each}
			</div>
		</div>
	{/if}
</div>
