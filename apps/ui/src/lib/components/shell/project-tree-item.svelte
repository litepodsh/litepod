<script lang="ts">
	import PlusIcon from '@lucide/svelte/icons/plus';
	import ChevronRightIcon from '@lucide/svelte/icons/chevron-right';
	import FlaskConicalIcon from '@lucide/svelte/icons/flask-conical';
	import FolderIcon from '@lucide/svelte/icons/folder';
	import FolderOpenIcon from '@lucide/svelte/icons/folder-open';
	import ShieldCheckIcon from '@lucide/svelte/icons/shield-check';
	import ApplicationIcon from '#lib/components/projects/application-icon.svelte';
	import * as Tooltip from '#lib/components/ui/tooltip/index.js';
	import TreeToggle from './tree-toggle.svelte';
	import * as Collapsible from '#lib/components/ui/collapsible/index.js';
	import * as Sidebar from '#lib/components/ui/sidebar/index.js';
	import { getProject, type EnvironmentDto, type ProjectDto } from '#lib/clients/project-client.js';
	import { t } from '#lib/i18n/index.svelte.js';
	import { isDisplayableImageUrl } from '#lib/utils/application-card-icon.js';
	import { isNamedIcon } from '#lib/utils/organization-icons.js';
	import OrganizationIcon from '#lib/components/organization-icon.svelte';
	import { preloadOnHover } from '#lib/utils/preload.js';

	let {
		project,
		path,
		onNavigate,
	}: { project: ProjectDto; path: string; onNavigate: () => void } = $props();

	const href = $derived(`/projects/${encodeURIComponent(project.id)}`);
	let open = $state(false);
	let environments = $state<EnvironmentDto[]>([]);
	let openEnvs = $state<Record<string, boolean>>({});
	let failedApplicationIds = $state(new Set<string>());

	// The branch containing the current page starts expanded.
	$effect(() => {
		if (path === href || path.startsWith(`${href}/`)) open = true;
	});

	// shortcut: loaded once per expand, so apps created elsewhere show after a reload.
	let loaded = false;
	$effect(() => {
		if (!open || loaded) return;
		loaded = true;
		getProject(project.organization_id, project.id)
			.then((detail) => (environments = detail.environments))
			.catch(() => (loaded = false));
	});

	const envOpen = (env: EnvironmentDto, eh: string) => openEnvs[env.id] ?? path.startsWith(eh);
	const inProject = $derived(path === href || path.startsWith(`${href}/`));

	// Inside the branch a click only folds or unfolds it; from elsewhere it navigates and opens it.
	function toggleOrGo(event: MouseEvent, inside: boolean, toggle: () => void, expand: () => void) {
		if (inside) {
			event.preventDefault();
			toggle();
		} else {
			expand();
			onNavigate();
		}
	}
	const envHref = (env: EnvironmentDto) => `${href}/environments/${encodeURIComponent(env.id)}`;
</script>

<Collapsible.Root bind:open>
	<Sidebar.MenuItem>
		<Sidebar.MenuButton isActive={path === href}>
			{#snippet child({ props })}
				<a {href} onclick={(event) => toggleOrGo(event, inProject, () => (open = !open), () => (open = true))} use:preloadOnHover={href} {...props}>
					{#if isNamedIcon(project.icon_url)}
						<OrganizationIcon icon={project.icon_url} class="size-4 shrink-0" />
					{:else if isDisplayableImageUrl(project.icon_url)}
						<img src={project.icon_url} alt="" class="size-4 shrink-0 object-contain" />
					{:else}
						{#if open}<FolderOpenIcon />{:else}<FolderIcon />{/if}
					{/if}
					<span>{project.name}</span>
				</a>
			{/snippet}
		</Sidebar.MenuButton>
		<TreeToggle
			label={open ? t('menu.sidebar.collapse') : t('menu.sidebar.expand')}
			class="absolute end-1 top-1.5 flex size-5 items-center justify-center rounded-md hover:bg-sidebar-accent">
			<ChevronRightIcon class="size-4 transition-transform {open ? 'rotate-90' : ''}" />
		</TreeToggle>
		<Collapsible.Content class="lp-collapsible">
			<Sidebar.MenuSub class="me-0 pe-0 border-transparent transition-colors hover:border-sidebar-border">
				{#each environments as env (env.id)}
					{@const eh = envHref(env)}
					<Collapsible.Root bind:open={() => openEnvs[env.id] ?? path.startsWith(eh), (v) => (openEnvs[env.id] = v)}>
						<Sidebar.MenuSubItem>
							<Sidebar.MenuSubButton isActive={path === eh}>
								{#snippet child({ props })}
									<a
										href={eh}
										onclick={(event) =>
											toggleOrGo(
												event,
												path === eh || path.startsWith(`${eh}/`),
												() => (openEnvs[env.id] = !envOpen(env, eh)),
												() => (openEnvs[env.id] = true),
											)}
										use:preloadOnHover={eh}
										{...props}
										class="{props.class} pe-12">
										{#if env.is_production}<ShieldCheckIcon />{:else}<FlaskConicalIcon />{/if}
										<span>{env.name}</span>
									</a>
								{/snippet}
							</Sidebar.MenuSubButton>
							<Tooltip.Root>
								<Tooltip.Trigger>
									{#snippet child({ props })}
										<a
											{...props}
											href="{eh}?new=app"
											onclick={onNavigate}
											aria-label={t('projects.application_form.create_application')}
											class="absolute end-6 top-1 flex size-5 items-center justify-center rounded-md hover:bg-sidebar-accent">
											<PlusIcon class="size-4" />
										</a>
									{/snippet}
								</Tooltip.Trigger>
								<Tooltip.Content side="right">
									{t('projects.application_form.create_application')}
								</Tooltip.Content>
							</Tooltip.Root>
							{#if env.applications.length}
								<TreeToggle
									label={envOpen(env, eh) ? t('menu.sidebar.collapse') : t('menu.sidebar.expand')}
									class="absolute end-1 top-1 flex size-5 items-center justify-center rounded-md hover:bg-sidebar-accent">
									<ChevronRightIcon
										class="size-4 transition-transform {envOpen(env, eh) ? 'rotate-90' : ''}" />
								</TreeToggle>
							{/if}
							<Collapsible.Content class="lp-collapsible">
								<Sidebar.MenuSub class="ms-2 me-0 pe-0 border-transparent transition-colors hover:border-sidebar-border">
									{#each env.applications as app (app.id)}
										{@const ah = `${eh}/applications/${encodeURIComponent(app.id)}`}
										<Sidebar.MenuSubItem>
											<Sidebar.MenuSubButton isActive={path === ah || path.startsWith(`${ah}/`)}>
												{#snippet child({ props })}
													<a href={ah} onclick={onNavigate} use:preloadOnHover={ah} {...props}>
														<ApplicationIcon
															templateKind={app.template_kind}
															catalogIconUrl={app.catalog_icon_url}
															iconUrl={app.icon_url}
															applicationId={app.id}
															bind:failedApplicationIds
															class="size-4" />
														<span>{app.name}</span>
													</a>
												{/snippet}
											</Sidebar.MenuSubButton>
										</Sidebar.MenuSubItem>
									{/each}
								</Sidebar.MenuSub>
							</Collapsible.Content>
						</Sidebar.MenuSubItem>
					</Collapsible.Root>
				{/each}
			</Sidebar.MenuSub>
		</Collapsible.Content>
	</Sidebar.MenuItem>
</Collapsible.Root>
