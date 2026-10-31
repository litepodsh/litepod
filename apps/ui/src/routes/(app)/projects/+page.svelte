<script lang="ts">
	import { brandedTitle } from '#lib/ee/branding/branding.svelte.js';
	import { onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import { listOrganizations, type OrganizationDto } from '#lib/clients/organization-client.js';
	import {
		createProject,
		deleteProject,
		getDeployment,
		getProject,
		listProjects,
		updateProject,
		uploadProjectIcon,
		type EnvironmentDto,
		type ProjectDto,
	} from '#lib/clients/project-client.js';
	import ProjectFormDialog from '#lib/components/projects/project-form-dialog.svelte';
	import DeleteResourceDialog from '#lib/components/projects/delete-resource-dialog.svelte';
	import ProjectCard from '#lib/components/custom/projects/project-card.svelte';
	import { getRequestSparklines, type Sparklines } from '#lib/clients/analytics-client.js';
	import { resolveSelectedOrganization } from '#lib/utils/selected-organization.js';
	import type { OrganizationApp } from '#lib/utils/organization-apps.js';
	import { toast } from '#lib/utils/toast.js';
	import { invalidateOnboarding } from '#lib/state/onboarding.svelte.js';
	import FolderKanbanIcon from '@lucide/svelte/icons/folder-kanban';
	import PlusIcon from '@lucide/svelte/icons/plus';
	import SearchIcon from '@lucide/svelte/icons/search';
	import { t } from '#lib/i18n/index.svelte.js';
	import { allowed } from '#lib/state/permissions.svelte.js';

	let organization = $state<OrganizationDto | null>(null);
	let projects = $state<ProjectDto[]>([]);
	let environments = $state<Record<string, EnvironmentDto[]>>({});
	let statuses = $state<Record<string, OrganizationApp['status']>>({});
	let sparklines = $state<Sparklines | null>(null);
	let loading = $state(true);
	let loadError = $state('');
	let query = $state('');
	let projectDialogOpen = $state(false);
	let editingProject = $state<ProjectDto | null>(null);
	let deleteDialogOpen = $state(false);
	let projectToDelete = $state<ProjectDto | null>(null);

	let needle = $derived(query.trim().toLowerCase());
	let visible = $derived(
		projects.filter(
			(project) =>
				!needle ||
				project.name.toLowerCase().includes(needle) ||
				project.description.toLowerCase().includes(needle) ||
				(environments[project.id] ?? []).some(
					(environment) =>
						environment.name.toLowerCase().includes(needle) ||
						environment.applications.some((app) => app.name.toLowerCase().includes(needle)),
				),
		),
	);

	onMount(() => {
		void loadPage();
		// The assistant fires this after a confirmed create, rename or delete,
		// so the list it just navigated to is not the one from before.
		const reload = () => void loadPage();
		window.addEventListener('projects-updated', reload);
		return () => window.removeEventListener('projects-updated', reload);
	});

	async function loadPage() {
		loading = true;
		loadError = '';
		try {
			const organizations = await listOrganizations();
			organization = resolveSelectedOrganization(organizations);
			projects = organization ? await listProjects(organization.id) : [];
			const details = organization
				? await Promise.all(projects.map((project) => getProject(organization!.id, project.id)))
				: [];
			environments = Object.fromEntries(
				details.map((detail) => [detail.project.id, detail.environments]),
			);
			if (organization) {
				void loadStatuses(organization.id);
				// One call for every card's chart; on failure the charts just stay hidden.
				getRequestSparklines(organization.id).then(
					(result) => (sparklines = result),
					() => (sparklines = null),
				);
			}
		} catch (error) {
			loadError = error instanceof Error ? error.message : t('projects.projects.unable_to_load');
			toast.error(loadError);
		} finally {
			loading = false;
		}
	}

	/** Status pips fill in after the cards are on screen; a failed read leaves a grey pip. */
	async function loadStatuses(organizationId: string) {
		const rows = Object.entries(environments).flatMap(([projectId, envs]) =>
			envs.flatMap((environment) =>
				environment.applications.map((app) => ({ projectId, environment, app })),
			),
		);
		const results = await Promise.allSettled(
			rows.map(({ projectId, environment, app }) =>
				getDeployment(organizationId, projectId, environment.id, app.id),
			),
		);
		statuses = Object.fromEntries(
			rows.map(({ app }, index) => {
				const result = results[index];
				return [app.id, result.status === 'fulfilled' ? result.value.status : null];
			}),
		);
	}

	function openCreateDialog() {
		editingProject = null;
		projectDialogOpen = true;
	}

	function openEditDialog(project: ProjectDto) {
		editingProject = project;
		projectDialogOpen = true;
	}

	function openDeleteDialog(project: ProjectDto) {
		projectToDelete = project;
		deleteDialogOpen = true;
	}

	async function handleSaveProject(input: {
		name: string;
		description?: string;
		icon_url?: string | null;
	}) {
		if (!organization) return;

		if (editingProject) {
			const updated = await updateProject(organization.id, editingProject.id, input);
			projects = projects.map((project) => (project.id === updated.id ? updated : project));
			toast.success(t('projects.projects.updated'));
			invalidateOnboarding();
			return;
		}

		const project = await createProject(organization.id, input);
		projects = [project, ...projects];
		toast.success(t('projects.projects.created'));
		invalidateOnboarding();
		await goto(`/projects/${encodeURIComponent(project.id)}`);
	}

	async function handleUploadProjectIcon(file: File) {
		if (!organization || !editingProject)
			throw new Error('Choose a project before uploading an icon.');
		const updated = await uploadProjectIcon(organization.id, editingProject.id, file);
		return updated.icon_url ?? '';
	}

	async function handleDeleteProject() {
		if (!organization || !projectToDelete) return;

		await deleteProject(organization.id, projectToDelete.id);
		projects = projects.filter((project) => project.id !== projectToDelete?.id);
		toast.success(t('projects.projects.deleted'));
		invalidateOnboarding();
		projectToDelete = null;
	}
</script>

<svelte:head>
	<title>{brandedTitle(t('projects.projects.title'))}</title>
</svelte:head>

<div class="flex flex-col gap-5 px-4 py-6 sm:px-6">
	<div class="flex flex-wrap items-end justify-between gap-4">
		<div class="min-w-0">
			<h1 class="text-xl font-semibold tracking-tight">{t('projects.projects.title')}</h1>
			<p class="text-sm text-muted-foreground">
				{organization
					? t('projects.projects.manage', { name: organization.name })
					: t('projects.projects.manage_without_organization')}
			</p>
		</div>
		{#if organization && !loadError}
			<div class="flex w-full flex-wrap items-center gap-2 sm:w-auto">
				{#if projects.length}
					<label
						class="lp-well relative flex min-w-0 flex-1 items-center rounded-full bg-muted/60 sm:w-64 sm:flex-none">
						<SearchIcon class="pointer-events-none absolute left-3 size-4 text-muted-foreground" />
						<Input
							bind:value={query}
							placeholder={t('projects.projects.search_all')}
							aria-label={t('projects.projects.search_all')}
							class="h-9 rounded-full border-0 bg-transparent pl-9 shadow-none" />
					</label>
				{/if}
				{#if allowed('project.create')}
					<Button onclick={openCreateDialog} class="max-sm:flex-1">
						<PlusIcon class="size-4" />
						{t('projects.projects.create')}
					</Button>
				{/if}
			</div>
		{/if}
	</div>

	{#if loading}
		<div
			class="grid grid-cols-1 gap-4 md:grid-cols-2 xl:grid-cols-3"
			role="status"
			aria-busy="true"
			aria-label={t('projects.projects.loading')}>
			{#each { length: 3 } as _, index (index)}
				<Skeleton class="h-64 rounded-2xl" />
			{/each}
		</div>
	{:else if loadError}
		<div class="rounded-2xl border border-destructive/30 bg-card p-6 text-sm text-destructive">
			{loadError}
		</div>
	{:else if !organization}
		<div
			class="flex flex-col items-start gap-3 rounded-2xl border border-dashed border-foreground/15 bg-card p-8">
			<h2 class="font-medium">{t('projects.projects.no_organization')}</h2>
			<p class="text-sm text-muted-foreground">
				{t('projects.projects.no_organization_hint')}
			</p>
			<Button variant="outline" href="/organizations">{t('common.home.view_organizations')}</Button>
		</div>
	{:else if projects.length === 0}
		<div
			class="flex min-h-64 flex-col items-center justify-center rounded-2xl border border-dashed border-foreground/15 bg-card p-8 text-center">
			<div class="lp-raised flex size-12 items-center justify-center rounded-xl border bg-muted">
				<FolderKanbanIcon class="size-5 text-muted-foreground" />
			</div>
			<h2 class="mt-4 font-medium">{t('projects.projects.empty')}</h2>
			<p class="mt-1 max-w-sm text-sm text-muted-foreground">
				{t('projects.projects.empty_hint', { name: organization.name })}
			</p>
			{#if allowed('project.create')}
				<Button class="mt-5" onclick={openCreateDialog}>
					<PlusIcon class="size-4" />
					{t('projects.projects.create')}
				</Button>
			{/if}
		</div>
	{:else if visible.length === 0}
		<p class="p-6 text-sm text-muted-foreground">{t('projects.projects.no_matches')}</p>
	{:else}
		<section
			aria-label={t('projects.projects.title')}
			class="grid grid-cols-1 gap-4 md:grid-cols-2 xl:grid-cols-3">
			{#each visible as project (project.id)}
				<ProjectCard
					{project}
					environments={environments[project.id]}
					{statuses}
					sparklines={sparklines ?? undefined}
					query={needle}
					onedit={allowed('project.update', { projectId: project.id }) ? openEditDialog : undefined}
					ondelete={allowed('project.delete', { projectId: project.id })
						? openDeleteDialog
						: undefined} />
			{/each}
		</section>
	{/if}
</div>

<ProjectFormDialog
	bind:open={projectDialogOpen}
	project={editingProject}
	onsave={handleSaveProject}
	onUploadIcon={handleUploadProjectIcon} />

<DeleteResourceDialog
	bind:open={deleteDialogOpen}
	resourceName={projectToDelete?.name ?? ''}
	resourceType="project"
	cascadeDescription="All of its environments and applications will also be deleted."
	ondelete={handleDeleteProject} />
