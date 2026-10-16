<script lang="ts">
	import { allowedHere } from '#lib/state/permissions.svelte.js';
	import * as Dialog from '#lib/components/ui/dialog/index.js';
	import AccessPanel from '#lib/components/access/access-panel.svelte';
	import ShieldIcon from '@lucide/svelte/icons/shield';
	import { onDestroy } from 'svelte';
	import { toneBadge, toneClass, toneDot } from '#lib/utils/status-tone.js';
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import { Button } from '#lib/components/ui/button/index.js';
	import ProjectFormDialog from '#lib/components/projects/project-form-dialog.svelte';
	import EnvironmentFormDialog from '#lib/components/projects/environment-form-dialog.svelte';
	import ApplicationFormDialog from '#lib/components/projects/application-form-dialog.svelte';
	import DatabaseTemplateDialog from '#lib/components/projects/database-template-dialog.svelte';
	import DeleteResourceDialog from '#lib/components/projects/delete-resource-dialog.svelte';
	import SearchableDropdown, {
		type SearchableDropdownItem,
	} from '#lib/components/custom/searchable-dropdown.svelte';
	import {
		createApplication,
		createEnvironment,
		deleteApplication,
		deployApplication,
		startApplication,
		stopApplication,
		getDeployment,
		type DeploymentDto,
		deleteEnvironment,
		deleteProject,
		getProject,
		listProjects,
		updateApplication,
		uploadApplicationIcon,
		updateEnvironment,
		updateProject,
		uploadProjectIcon,
		type ApplicationDto,
		type EnvironmentDto,
		type ProjectDetailDto,
		type ProjectDto,
		type ServerSummaryDto,
		listReadyServers,
	} from '#lib/clients/project-client.js';
	import { getSystemVersion, type SystemVersion } from '#lib/clients/system-client.js';
	import { createCatalogTemplateApplication } from '#lib/clients/template-client.js';
	import ApplicationIcon from '#lib/components/projects/application-icon.svelte';
	import ProjectIcon from '#lib/components/projects/project-icon.svelte';
	import { isDisplayableImageUrl } from '#lib/utils/application-card-icon.js';
	import { listOrganizations } from '#lib/clients/organization-client.js';
	import {
		resolveSelectedOrganization,
		setSelectedOrganizationId,
	} from '#lib/utils/selected-organization.js';
	import { toast } from '#lib/utils/toast.js';
	import { invalidateOnboarding } from '#lib/state/onboarding.svelte.js';
	import { setPageBreadcrumbs } from '#lib/state/page-header.svelte.js';
	import { clearScopeHero, setScopeHero } from '#lib/state/shell.svelte.js';
	import CellChart from '#lib/components/custom/cell-chart.svelte';
	import {
		compactNumber,
		getRequestSparklines,
		sparklineTime,
		type Sparklines,
	} from '#lib/clients/analytics-client.js';
	import { environmentApplicationsHref, resolveEnvironment } from '#lib/utils/environment-route.js';
	import { deploymentTargetNotice } from '#lib/utils/server-visibility.js';
	import ServerMark from '#lib/components/servers/server-mark.svelte';
	import ServerGroupMark from '#lib/components/server-groups/server-group-mark.svelte';
	import BoxesIcon from '@lucide/svelte/icons/boxes';
	import FolderKanbanIcon from '@lucide/svelte/icons/folder-kanban';
	import LayersIcon from '@lucide/svelte/icons/layers';
	import EllipsisIcon from '@lucide/svelte/icons/ellipsis';
	import PlusIcon from '@lucide/svelte/icons/plus';
	import Trash2Icon from '@lucide/svelte/icons/trash-2';
	import PencilIcon from '@lucide/svelte/icons/pencil';
	import AppWindowIcon from '@lucide/svelte/icons/app-window';
	import DatabaseIcon from '@lucide/svelte/icons/database';
	import TriangleAlertIcon from '@lucide/svelte/icons/triangle-alert';
	import PlayIcon from '@lucide/svelte/icons/play';
	import SquareIcon from '@lucide/svelte/icons/square';
	import RotateCwIcon from '@lucide/svelte/icons/rotate-cw';
	import * as DropdownMenu from '#lib/components/ui/dropdown-menu/index.js';
	import SoftCheckbox from '#lib/components/custom/soft-checkbox.svelte';
	import { i18n, t } from '#lib/i18n/index.svelte.js';

	const projectId = $derived(page.params.project_id ?? '');
	const environmentReference = $derived(page.params.environment_id ?? '');

	let organizationId = $state<string | null>(null);
	let accessOpen = $state(false);
	let projects = $state<ProjectDto[]>([]);
	let detail = $state<ProjectDetailDto | null>(null);
	let environment = $state<EnvironmentDto | null>(null);
	let loading = $state(true);
	let error = $state<string | null>(null);
	let cancelled = false;
	let projectDialogOpen = $state(false);
	let deleteDialogOpen = $state(false);
	let environmentDialogOpen = $state(false);
	let applicationDialogOpen = $state(false);
	let applicationCreateMode = $state<'blank' | 'catalog'>('blank');
	let databaseTemplateDialogOpen = $state(false);
	let editingEnvironment = $state<EnvironmentDto | null>(null);
	let editingApplication = $state<ApplicationDto | null>(null);
	let readyServers = $state<ServerSummaryDto[]>([]);
	let systemVersion = $state<SystemVersion | null>(null);
	let failedIconApplicationIds = $state<Set<string>>(new Set());
	let projectIconFailed = $state(false);
	$effect(() => {
		// Reset the fallback when the project (or its icon) changes.
		void detail?.project.icon_url;
		projectIconFailed = false;
	});
	let applicationStatus = $state<Record<string, DeploymentDto['status']>>({});
	let sparklines = $state<Sparklines | null>(null);
	let hoveredRequests = $state<{ id: string; index: number } | null>(null);
	let selectMode = $state(false);
	let selectedIds = $state<Set<string>>(new Set());
	let bulkBusy = $state(false);
	let bulkDeleteDialogOpen = $state(false);
	let paintValue: boolean | null = null;
	// Anchor for arrow-key selection: index into the current environment's applications.
	let cursorIndex = $state<number | null>(null);

	function typingInField() {
		const el = document.activeElement as HTMLElement | null;
		return !!el && (el.tagName === 'INPUT' || el.tagName === 'TEXTAREA' || el.isContentEditable);
	}

	// Column count of the card grid (`sm:grid-cols-2 xl:grid-cols-3`).
	function gridCols() {
		if (window.matchMedia('(min-width: 1280px)').matches) return 3;
		if (window.matchMedia('(min-width: 640px)').matches) return 2;
		return 1;
	}

	function markCursor(id: string) {
		const idx = environment?.applications.findIndex((application) => application.id === id) ?? -1;
		if (idx >= 0) cursorIndex = idx;
	}

	const DATABASE_KINDS = new Set([
		'postgres',
		'mariadb',
		'mongodb',
		'dragonflydb',
		'keydb',
		'sqlserver',
	]);

	const statusMeta: Record<string, { dot: string; label: string }> = {
		running: { dot: toneDot('ok'), label: t('projects.environment.running') },
		// The pulse is what reads as "in progress"; the colour only says "not settled".
		deploying: {
			dot: `${toneDot('warn')} animate-pulse`,
			label: t('projects.environment.deploying'),
		},
		failed: { dot: toneDot('danger'), label: t('projects.environment.failed') },
		stopped: { dot: toneDot('neutral'), label: t('projects.environment.stopped') },
		ready: { dot: toneDot('info'), label: t('projects.environment.ready') },
		not_deployed: { dot: toneDot('neutral'), label: t('projects.environment.not_deployed') },
	};

	const allSelected = $derived(
		!!environment &&
			environment.applications.length > 0 &&
			environment.applications.every((application) => selectedIds.has(application.id)),
	);

	function toggleSelected(id: string) {
		const next = new Set(selectedIds);
		if (next.has(id)) next.delete(id);
		else next.add(id);
		selectedIds = next;
		markCursor(id);
	}

	function setSelected(id: string, on: boolean) {
		const next = new Set(selectedIds);
		if (on) next.add(id);
		else next.delete(id);
		selectedIds = next;
		if (on) markCursor(id);
	}

	function toggleSelectAll() {
		if (!environment) return;
		selectedIds = allSelected
			? new Set()
			: new Set(environment.applications.map((application) => application.id));
	}

	// ponytail: Ctrl/⌘+hover paints a selection like a file explorer. First card touched
	// in the hold decides whether the pass selects or deselects; releasing the key resets.
	function paintEnter(event: MouseEvent, id: string) {
		if (!selectMode || !(event.ctrlKey || event.metaKey)) return;
		if (paintValue === null) paintValue = !selectedIds.has(id);
		setSelected(id, paintValue);
	}

	$effect(() => {
		if (!selectMode) {
			selectedIds = new Set();
			paintValue = null;
			cursorIndex = null;
			return;
		}
		const onKeyUp = (event: KeyboardEvent) => {
			if (event.key === 'Control' || event.key === 'Meta') paintValue = null;
		};
		window.addEventListener('keyup', onKeyUp);
		return () => window.removeEventListener('keyup', onKeyUp);
	});

	// While multi-select is on: arrow keys extend the selection across the grid from
	// the last-touched card (like Shift + arrows in a file manager); Esc leaves the mode.
	$effect(() => {
		const DIRS: Record<string, (cols: number) => number> = {
			ArrowRight: () => 1,
			ArrowLeft: () => -1,
			ArrowDown: (cols) => cols,
			ArrowUp: (cols) => -cols,
		};
		const onKeyDown = (event: KeyboardEvent) => {
			if (!selectMode) return;
			// Esc leaves multi-select (unless a dialog is open — let it close that first).
			if (event.key === 'Escape') {
				if (!document.querySelector('[role="dialog"][data-state="open"]')) selectMode = false;
				return;
			}
			if (!(event.key in DIRS) || typingInField()) return;
			const apps = environment?.applications ?? [];
			if (apps.length === 0 || selectedIds.size === 0) return;
			let from = cursorIndex;
			if (from === null || from < 0 || from >= apps.length) {
				from = apps.reduce((last, app, i) => (selectedIds.has(app.id) ? i : last), -1);
			}
			if (from < 0) return;
			const next = Math.min(apps.length - 1, Math.max(0, from + DIRS[event.key](gridCols())));
			if (next === from) return;
			event.preventDefault();
			setSelected(apps[next].id, true);
			cursorIndex = next;
			document.querySelector(`[data-app-index="${next}"]`)?.scrollIntoView({ block: 'nearest' });
		};
		window.addEventListener('keydown', onKeyDown);
		return () => window.removeEventListener('keydown', onKeyDown);
	});

	// Hold Ctrl (or ⌘ on macOS) for a moment on this page to auto-enable multi-select.
	$effect(() => {
		const HOLD_MS = 550;
		let holdTimer: ReturnType<typeof setTimeout> | null = null;
		const clear = () => {
			if (holdTimer) clearTimeout(holdTimer);
			holdTimer = null;
		};
		const isModifier = (key: string) => key === 'Control' || key === 'Meta';
		const onKeyDown = (event: KeyboardEvent) => {
			if (event.repeat) return;
			if (!isModifier(event.key)) {
				clear(); // another key = a real shortcut (Ctrl+C/A/…), not a hold
				return;
			}
			if (selectMode || holdTimer || typingInField()) return;
			if (!environment || environment.applications.length === 0) return;
			holdTimer = setTimeout(() => {
				holdTimer = null;
				selectMode = true;
			}, HOLD_MS);
		};
		const onKeyUp = (event: KeyboardEvent) => {
			if (isModifier(event.key)) clear();
		};
		window.addEventListener('keydown', onKeyDown);
		window.addEventListener('keyup', onKeyUp);
		window.addEventListener('blur', clear);
		return () => {
			clear();
			window.removeEventListener('keydown', onKeyDown);
			window.removeEventListener('keyup', onKeyUp);
			window.removeEventListener('blur', clear);
		};
	});

	async function refreshStatuses(env: EnvironmentDto) {
		if (!organizationId) return;
		const results = await Promise.allSettled(
			env.applications.map((application) =>
				getDeployment(organizationId!, projectId, env.id, application.id),
			),
		);
		if (cancelled) return;
		const next: Record<string, DeploymentDto['status']> = {};
		results.forEach((result, index) => {
			if (result.status === 'fulfilled') next[env.applications[index].id] = result.value.status;
		});
		applicationStatus = next;
	}

	async function runBulk(action: 'start' | 'stop' | 'redeploy') {
		if (!organizationId || !environment || selectedIds.size === 0) return;
		const env = environment;
		const ids = [...selectedIds];
		const fn =
			action === 'start'
				? startApplication
				: action === 'stop'
					? stopApplication
					: deployApplication;
		bulkBusy = true;
		try {
			const results = await Promise.allSettled(
				ids.map((id) => fn(organizationId!, projectId, env.id, id)),
			);
			const failed = results.filter((result) => result.status === 'rejected').length;
			const verb =
				action === 'redeploy'
					? t('projects.environment.redeploy')
					: action === 'start'
						? t('projects.environment.start')
						: t('projects.environment.stop');
			if (failed)
				toast.error(
					t('projects.environment.bulk_toast_failed', {
						verb,
						succeeded: ids.length - failed,
						failed,
					}),
				);
			else
				toast.success(
					t(`projects.environment.bulk_toast_success_${ids.length === 1 ? 'one' : 'many'}`, {
						verb,
						count: ids.length,
					}),
				);
		} finally {
			bulkBusy = false;
		}
		selectMode = false;
		await loadProject(projectId, environmentReference);
	}

	async function handleBulkDelete() {
		if (!organizationId || !environment)
			throw new Error(t('projects.environment.choose_environment_before_delete'));
		const env = environment;
		const ids = [...selectedIds];
		const results = await Promise.allSettled(
			ids.map((id) => deleteApplication(organizationId!, projectId, env.id, id)),
		);
		const failed = results.filter((result) => result.status === 'rejected').length;
		if (failed)
			toast.error(
				t('projects.environment.bulk_delete_toast_failed', {
					succeeded: ids.length - failed,
					failed,
				}),
			);
		else
			toast.success(
				t(`projects.environment.bulk_delete_toast_success_${ids.length === 1 ? 'one' : 'many'}`, {
					count: ids.length,
				}),
			);
		selectMode = false;
		await loadProject(projectId, environmentReference);
	}
	let deleteTarget = $state<{
		type: 'project' | 'environment' | 'application';
		name: string;
		environment?: EnvironmentDto;
		application?: ApplicationDto;
	} | null>(null);
	let environmentItems = $derived<SearchableDropdownItem[]>(
		detail?.environments.map((item) => ({
			value: item.id,
			label: item.name,
			description: item.description || (item.is_production ? 'prod' : undefined),
		})) ?? [],
	);
	let projectItems = $derived<SearchableDropdownItem[]>(
		projects.map((item) => ({
			value: item.id,
			label: item.name,
			description: item.description || undefined,
			icon: item.id,
		})),
	);
	const deploymentNotice = $derived(
		deploymentTargetNotice(systemVersion?.is_cloud ?? false, readyServers.length),
	);

	function switchEnvironment(environmentId: string) {
		const target = detail?.environments.find((item) => item.id === environmentId);
		if (!target || target.id === environment?.id) return;
		void goto(
			`/projects/${encodeURIComponent(projectId)}/environments/${encodeURIComponent(target.name)}`,
		);
	}

	function switchProject(newProjectId: string) {
		void goto(`/projects/${encodeURIComponent(newProjectId)}`);
	}

	async function loadProject(pId: string, ref: string) {
		if (!organizationId) return;
		loading = true;
		error = null;
		try {
			const [projectDetail, projectList] = await Promise.all([
				getProject(organizationId, pId),
				listProjects(organizationId),
			]);
			if (cancelled) return;
			const resolvedEnvironment = resolveEnvironment(projectDetail, ref);
			if (!resolvedEnvironment) {
				error = t('projects.common.no_environment_found');
				return;
			}
			detail = projectDetail;
			projects = projectList;
			environment = resolvedEnvironment;
			void refreshStatuses(resolvedEnvironment);
			// One call for every app card's 24h chart; on failure the charts stay hidden.
			getRequestSparklines(organizationId).then(
				(result) => (sparklines = cancelled ? sparklines : result),
				() => {},
			);
			setPageBreadcrumbs([
				{ label: t('projects.common.projects'), href: '/projects', icon: FolderKanbanIcon },
				{
					label: projectDetail.project.name,
					iconSnippet: projectCrumbIcon,
					switcher: {
						items: projectItems,
						selectedValue: pId,
						onSelect: switchProject,
						itemIcon: projectItemIcon,
						label: t('projects.common.select_project'),
						searchPlaceholder: t('projects.common.search_projects'),
						emptyText: t('projects.common.no_project_found'),
					},
				},
				{
					label: resolvedEnvironment.name,
					icon: LayersIcon,
					switcher: {
						items: environmentItems,
						selectedValue: resolvedEnvironment.id,
						onSelect: switchEnvironment,
						label: t('projects.common.select_environment'),
						searchPlaceholder: t('projects.common.search_environments'),
						emptyText: t('projects.common.no_environment_found'),
					},
				},
			]);
		} catch (caught) {
			if (cancelled) return;
			detail = null;
			environment = null;
			error = caught instanceof Error ? caught.message : t('projects.projects.unable_to_load');
		} finally {
			if (!cancelled) loading = false;
		}
	}

	$effect(() => {
		const pId = projectId;
		const ref = environmentReference;
		if (!pId || !ref) return;
		void (async () => {
			if (!organizationId) {
				try {
					const organization = resolveSelectedOrganization(await listOrganizations());
					if (cancelled) return;
					if (!organization) {
						error = t('projects.projects.choose_organization');
						loading = false;
						return;
					}
					organizationId = organization.id;
					setSelectedOrganizationId(organization.id);
				} catch (caught) {
					if (cancelled) return;
					error =
						caught instanceof Error
							? caught.message
							: t('projects.environment.load_organizations_failed');
					loading = false;
					return;
				}
			}
			await loadProject(pId, ref);
			if (cancelled) return;
			const [version, servers] = await Promise.all([
				getSystemVersion().catch(() => null),
				listReadyServers(organizationId!).catch(() => []),
			]);
			if (cancelled) return;
			systemVersion = version;
			readyServers = servers;
		})();
	});

	// The assistant fires this after a confirmed create or delete that has no
	// application to name - a new application in this environment, among others.
	$effect(() => {
		const reload = () => void loadProject(projectId, environmentReference);
		window.addEventListener('projects-updated', reload);
		return () => window.removeEventListener('projects-updated', reload);
	});

	onDestroy(() => {
		cancelled = true;
		clearScopeHero();
	});

	// The project's info card lives in the shell, above the tabs.
	$effect(() => {
		if (!detail || !environment) return;
		const apps = environment.applications;
		const databases = apps.filter((app) => DATABASE_KINDS.has(app.template_kind ?? '')).length;
		const running = apps.filter((app) => applicationStatus[app.id] === 'running').length;
		setScopeHero({
			title: detail.project.name,
			avatar: heroAvatar,
			meta: detail.project.description ? heroMeta : undefined,
			actions: heroActions,
			stats: [
				{ label: t('projects.environment.applications'), value: String(apps.length - databases) },
				{ label: t('projects.environment.databases'), value: String(databases) },
				{ label: t('projects.environment.running'), value: `${running}/${apps.length}` },
				{
					label: t('projects.environment.environments'),
					value: String(detail.environments.length),
				},
			],
		});
	});

	async function handleSaveProject(input: {
		name: string;
		description?: string;
		icon_url?: string | null;
	}) {
		if (!organizationId)
			throw new Error(t('projects.environment.choose_organization_before_save_project'));
		await updateProject(organizationId, projectId, input);
		await loadProject(projectId, environmentReference);
		toast.success(t('projects.projects.updated'));
	}

	async function handleUploadProjectIcon(file: File) {
		if (!organizationId)
			throw new Error(t('projects.environment.choose_organization_before_upload_icon'));
		const updated = await uploadProjectIcon(organizationId, projectId, file);
		return updated.icon_url ?? '';
	}

	async function handleSaveEnvironment(input: { name: string; description: string }) {
		if (!organizationId)
			throw new Error(t('projects.environment.choose_organization_before_save_environment'));
		if (editingEnvironment) {
			await updateEnvironment(organizationId, projectId, editingEnvironment.id, input);
			toast.success(t('projects.environment.environment_updated'));
		} else {
			await createEnvironment(organizationId, projectId, input);
			toast.success(t('projects.environment.environment_created'));
		}
		await loadProject(projectId, environmentReference);
	}

	async function handleSaveApplication(input: {
		name: string;
		description: string;
		server_id?: string;
		server_group_id?: string;
		icon_url?: string | null;
	}) {
		if (!organizationId || !environment)
			throw new Error(t('projects.environment.choose_environment_before_save_application'));
		if (editingApplication) {
			await updateApplication(
				organizationId,
				projectId,
				environment.id,
				editingApplication.id,
				input,
			);
			toast.success(t('projects.environment.application_updated'));
			invalidateOnboarding();
		} else {
			const app = await createApplication(organizationId, projectId, environment.id, input);
			toast.success(t('projects.environment.application_created'));
			invalidateOnboarding();
			await loadProject(projectId, environmentReference);
			await goto(
				`${environmentApplicationsHref(projectId, environment.name)}/${encodeURIComponent(app.id)}`,
			);
			return;
		}
		await loadProject(projectId, environmentReference);
	}

	async function handleUploadApplicationIcon(file: File) {
		if (!organizationId || !environment || !editingApplication)
			throw new Error(t('projects.environment.choose_application_before_upload_icon'));
		const updated = await uploadApplicationIcon(
			organizationId,
			projectId,
			environment.id,
			editingApplication.id,
			file,
		);
		return updated.icon_url ?? '';
	}

	async function handleCreateCatalogTemplate(templateId: string, serverId: string) {
		if (!organizationId || !environment)
			throw new Error(t('projects.environment.choose_environment_before_create_application'));
		const application = await createCatalogTemplateApplication(
			organizationId,
			projectId,
			environment.id,
			templateId,
			serverId,
		);
		await loadProject(projectId, environmentReference);
		toast.success(t('projects.environment.application_created_review'));
		await goto(
			`${environmentApplicationsHref(projectId, environment.name)}/${encodeURIComponent(application.id)}`,
		);
	}

	function openCreateEnvironment() {
		editingEnvironment = null;
		environmentDialogOpen = true;
	}

	function openEditEnvironment(target: EnvironmentDto) {
		editingEnvironment = target;
		environmentDialogOpen = true;
	}

	async function openCreateApplication() {
		if (deploymentNotice) {
			toast.info(deploymentNotice);
			return;
		}
		editingApplication = null;
		applicationCreateMode = 'blank';
		applicationDialogOpen = true;
		try {
			readyServers = organizationId ? await listReadyServers(organizationId) : [];
		} catch (caught) {
			toast.error(
				caught instanceof Error
					? caught.message
					: t('projects.environment.failed_load_ready_servers'),
			);
		}
	}

	function openCreateCatalogTemplate() {
		if (deploymentNotice) {
			toast.info(deploymentNotice);
			return;
		}
		editingApplication = null;
		applicationCreateMode = 'catalog';
		applicationDialogOpen = true;
		if (organizationId)
			void listReadyServers(organizationId)
				.then((servers) => (readyServers = servers))
				.catch((caught) =>
					toast.error(
						caught instanceof Error
							? caught.message
							: t('projects.environment.failed_load_ready_servers'),
					),
				);
	}

	function openCreateDatabase() {
		if (deploymentNotice) {
			toast.info(deploymentNotice);
			return;
		}
		databaseTemplateDialogOpen = true;
		if (organizationId)
			void listReadyServers(organizationId)
				.then((servers) => (readyServers = servers))
				.catch((caught) =>
					toast.error(
						caught instanceof Error
							? caught.message
							: t('projects.environment.failed_load_ready_servers'),
					),
				);
	}

	async function handlePostgresCreated(created: { application: ApplicationDto }) {
		if (!environment) return;
		await loadProject(projectId, environmentReference);
		toast.success(t('projects.environment.created_postgres'));
		await goto(
			`${environmentApplicationsHref(projectId, environment.name)}/${encodeURIComponent(created.application.id)}`,
		);
	}

	async function handleMariaDbCreated(created: { application: ApplicationDto }) {
		if (!environment) return;
		await loadProject(projectId, environmentReference);
		toast.success(t('projects.environment.created_mariadb'));
		await goto(
			`${environmentApplicationsHref(projectId, environment.name)}/${encodeURIComponent(created.application.id)}`,
		);
	}

	async function handleDragonflyDbCreated(created: { application: ApplicationDto }) {
		if (!environment) return;
		await loadProject(projectId, environmentReference);
		toast.success(t('projects.environment.created_dragonfly'));
		await goto(
			`${environmentApplicationsHref(projectId, environment.name)}/${encodeURIComponent(created.application.id)}`,
		);
	}

	async function handleKeyDbCreated(created: { application: ApplicationDto }) {
		if (!environment) return;
		await loadProject(projectId, environmentReference);
		toast.success(t('projects.environment.created_keydb'));
		await goto(
			`${environmentApplicationsHref(projectId, environment.name)}/${encodeURIComponent(created.application.id)}`,
		);
	}

	async function handleSqlServerCreated(created: { application: ApplicationDto }) {
		if (!environment) return;
		await loadProject(projectId, environmentReference);
		toast.success(t('projects.environment.created_sqlserver'));
		await goto(
			`${environmentApplicationsHref(projectId, environment.name)}/${encodeURIComponent(created.application.id)}`,
		);
	}

	async function handleMongoDbCreated(created: { application: ApplicationDto }) {
		if (!environment) return;
		await loadProject(projectId, environmentReference);
		toast.success(t('projects.environment.created_mongodb'));
		await goto(
			`${environmentApplicationsHref(projectId, environment.name)}/${encodeURIComponent(created.application.id)}`,
		);
	}

	function openEditApplication(application: ApplicationDto) {
		editingApplication = application;
		applicationDialogOpen = true;
	}

	function confirmDeleteEnvironment(target: EnvironmentDto) {
		deleteTarget = { type: 'environment', name: target.name, environment: target };
		deleteDialogOpen = true;
	}

	function confirmDeleteApplication(target: EnvironmentDto, application: ApplicationDto) {
		deleteTarget = {
			type: 'application',
			name: application.name,
			environment: target,
			application,
		};
		deleteDialogOpen = true;
	}

	async function handleDelete() {
		if (!organizationId || !deleteTarget)
			throw new Error(t('projects.environment.nothing_selected_for_deletion'));
		if (deleteTarget.type === 'project') {
			await deleteProject(organizationId, projectId);
			toast.success(t('projects.projects.deleted'));
			window.location.assign('/projects');
			return;
		}
		if (deleteTarget.type === 'environment' && deleteTarget.environment) {
			await deleteEnvironment(organizationId, projectId, deleteTarget.environment.id);
			toast.success(t('projects.environment.environment_deleted'));
			await goto(`/projects/${encodeURIComponent(projectId)}`);
			return;
		}
		if (
			deleteTarget.type === 'application' &&
			deleteTarget.environment &&
			deleteTarget.application
		) {
			await deleteApplication(
				organizationId,
				projectId,
				deleteTarget.environment.id,
				deleteTarget.application.id,
			);
			toast.success(t('projects.environment.application_deleted'));
		}
		await loadProject(projectId, environmentReference);
	}

	function requestProjectDelete() {
		deleteTarget = { type: 'project', name: detail?.project.name ?? '' };
		deleteDialogOpen = true;
	}

	function deleteCascadeDescription() {
		if (deleteTarget?.type === 'project') return t('projects.environment.cascade_project');
		if (deleteTarget?.type === 'environment') return t('projects.environment.cascade_environment');
		return t('projects.environment.cascade_application');
	}
</script>

{#snippet heroAvatar()}
	<span class="grid size-full place-items-center bg-muted">
		{#if isDisplayableImageUrl(detail?.project.icon_url) && !projectIconFailed}
			<img
				src={detail?.project.icon_url}
				alt=""
				class="size-full object-contain"
				onerror={() => (projectIconFailed = true)} />
		{:else}
			<FolderKanbanIcon class="size-5" />
		{/if}
	</span>
{/snippet}

{#snippet heroMeta()}
	<span class="max-w-2xl">{detail?.project.description}</span>
{/snippet}

{#snippet heroActions()}
	<Button variant="outline" size="sm" onclick={() => (accessOpen = true)}
		><ShieldIcon />{t('organizations.access.access')}</Button>
	<Button variant="outline" size="sm" onclick={() => (projectDialogOpen = true)}
		><PencilIcon />{t('projects.environment.edit_project')}</Button>
	<Button
		variant="ghost"
		size="icon-sm"
		class="text-destructive hover:text-destructive"
		aria-label={t('projects.common.delete')}
		onclick={requestProjectDelete}><Trash2Icon /></Button>
{/snippet}

{#snippet projectItemIcon(id: string)}
	<ProjectIcon iconUrl={projects.find((item) => item.id === id)?.icon_url} />
{/snippet}

{#snippet projectCrumbIcon()}
	{#if isDisplayableImageUrl(detail?.project.icon_url) && !projectIconFailed}
		<img
			src={detail?.project.icon_url}
			alt=""
			class="size-4 rounded-[4px] object-contain"
			onerror={() => (projectIconFailed = true)} />
	{:else}
		<FolderKanbanIcon class="text-muted-foreground size-3.5" />
	{/if}
{/snippet}

<div class="flex w-full flex-col gap-6 px-4 py-6 sm:px-6">
	{#if loading}
		<p class="text-muted-foreground text-sm">{t('projects.common.loading_project')}</p>
	{:else if error}
		<div class="rounded-lg border border-destructive/30 bg-destructive/5 p-4">
			<p class="font-medium">{t('projects.projects.unable_to_load')}</p>
			<p class="mt-1 text-sm text-muted-foreground">{error}</p>
		</div>
	{:else if detail && environment}
		{@const currentEnvironment = environment}
		{@const databaseApplications = currentEnvironment.applications.filter((application) =>
			DATABASE_KINDS.has(application.template_kind ?? ''),
		)}
		{@const regularApplications = currentEnvironment.applications.filter(
			(application) => !DATABASE_KINDS.has(application.template_kind ?? ''),
		)}

		{#if deploymentNotice}
			<div
				class={`flex flex-wrap items-center justify-between gap-3 rounded-lg border p-3 text-sm ${toneBadge('warn')}`}
				role="alert">
				<span class="flex items-center gap-2">
					<TriangleAlertIcon class="size-4 shrink-0" />{deploymentNotice}
				</span>
				{#if organizationId}
					<Button
						variant="outline"
						size="sm"
						href={`/organizations/${encodeURIComponent(organizationId)}/servers`}
						>{t('projects.environment.connect_server')}</Button>
				{/if}
			</div>
		{/if}

		<section class="flex flex-col gap-5">
			<div class="flex flex-col gap-4 sm:flex-row sm:items-end sm:justify-between">
				<div class="min-w-0">
					<h2 class="text-lg font-semibold">{t('projects.environment.applications')}</h2>
					<p class="text-sm text-muted-foreground">
						{t('projects.environment.choose_environment')}
					</p>
				</div>
				<Button variant="outline" onclick={openCreateEnvironment}
					><PlusIcon class="size-4" />{t('projects.environment.add_environment')}</Button>
			</div>

			<div
				class="lp-lifted flex flex-col gap-3 rounded-[min(var(--radius-4xl),28px)] bg-card p-5 ring-1 ring-foreground/8 sm:flex-row sm:items-center sm:justify-between">
				<div class="flex min-w-0 items-center gap-3">
					<SearchableDropdown
						items={environmentItems}
						selectedValue={currentEnvironment.id}
						onSelect={switchEnvironment}
						label={t('projects.common.select_environment')}
						searchPlaceholder={t('projects.common.search_environments')}
						emptyText={t('projects.common.no_environment_found')} />
					{#if currentEnvironment.is_production}<span
							class={`rounded-full px-2 py-0.5 text-xs font-medium ${toneClass('ok')}`}>prod</span
						>{/if}
				</div>
				<div class="flex gap-2">
					<DropdownMenu.Root>
						<DropdownMenu.Trigger
							>{#snippet child({ props })}<Button
									{...props}
									variant="outline"
									size="icon"
									aria-label={t('projects.common.actions_for', { name: currentEnvironment.name })}
									><EllipsisIcon class="size-4" /></Button
								>{/snippet}</DropdownMenu.Trigger>
						<DropdownMenu.Content align="end">
							{#if !currentEnvironment.is_production}<DropdownMenu.Item
									onclick={() => openEditEnvironment(currentEnvironment)}
									><PencilIcon class="size-4" />{t(
										'projects.environment.edit_environment',
									)}</DropdownMenu.Item
								>{/if}
							{#if !currentEnvironment.is_production}<DropdownMenu.Item
									class="text-destructive focus:text-destructive"
									onclick={() => confirmDeleteEnvironment(currentEnvironment)}
									><Trash2Icon class="size-4" />{t(
										'projects.environment.delete_environment',
									)}</DropdownMenu.Item
								>{/if}
						</DropdownMenu.Content>
					</DropdownMenu.Root>
					<DropdownMenu.Root>
						<DropdownMenu.Trigger
							>{#snippet child({ props })}<Button {...props} disabled={Boolean(deploymentNotice)}
									><PlusIcon class="size-4" />{t('projects.environment.new_resource')}</Button
								>{/snippet}</DropdownMenu.Trigger>
						<DropdownMenu.Content align="end">
							<DropdownMenu.Item
								onclick={openCreateApplication}
								disabled={Boolean(deploymentNotice)}
								><AppWindowIcon class="size-4" />{t(
									'projects.environment.application',
								)}</DropdownMenu.Item>
							<DropdownMenu.Item
								onclick={openCreateCatalogTemplate}
								disabled={Boolean(deploymentNotice)}
								><BoxesIcon class="size-4" />{t(
									'projects.environment.template',
								)}</DropdownMenu.Item>
							<DropdownMenu.Item onclick={openCreateDatabase} disabled={Boolean(deploymentNotice)}
								><DatabaseIcon class="size-4" />{t(
									'projects.environment.database',
								)}</DropdownMenu.Item>
						</DropdownMenu.Content>
					</DropdownMenu.Root>
				</div>
			</div>

			{#if currentEnvironment.applications.length > 0}
				<div class="flex min-h-9 flex-wrap items-center gap-2">
					<label class="flex cursor-pointer items-center gap-2 text-sm font-medium select-none">
						<SoftCheckbox
							bind:checked={selectMode}
							aria-label={t('projects.environment.enable_multiple_select')} />
						{t('projects.environment.select_multiple')}
					</label>
					{#if selectMode}
						<Button variant="ghost" size="sm" onclick={toggleSelectAll}>
							{allSelected ? t('projects.common.clear') : t('projects.environment.select_all')}
						</Button>
						<span class="text-xs text-muted-foreground"
							>{t('projects.environment.selected', { count: selectedIds.size })}</span>
						<div class="ml-auto flex flex-wrap gap-2">
							{#if allowedHere('app.deploy')}
								<Button
									variant="outline"
									size="sm"
									disabled={selectedIds.size === 0 || bulkBusy}
									onclick={() => runBulk('start')}
									><PlayIcon class="size-4" />{t('projects.environment.start')}</Button>
								<Button
									variant="outline"
									size="sm"
									disabled={selectedIds.size === 0 || bulkBusy}
									onclick={() => runBulk('stop')}
									><SquareIcon class="size-4" />{t('projects.environment.stop')}</Button>
								<Button
									variant="outline"
									size="sm"
									disabled={selectedIds.size === 0 || bulkBusy}
									onclick={() => runBulk('redeploy')}
									><RotateCwIcon class="size-4" />{t('projects.environment.redeploy')}</Button>
							{/if}
							<Button
								variant="outline"
								size="sm"
								class="text-destructive hover:text-destructive"
								disabled={selectedIds.size === 0 || bulkBusy}
								onclick={() => (bulkDeleteDialogOpen = true)}
								><Trash2Icon class="size-4" />{t('projects.common.delete')}</Button>
						</div>
					{/if}
				</div>
				<p class="-mt-2 min-h-8 text-xs text-muted-foreground">
					{#if selectMode}
						{t('projects.environment.paint_hint_1')}
						<kbd class="rounded border px-1 py-0.5 font-mono text-[10px]">Ctrl</kbd>
						{t('projects.environment.paint_hint_2')}
						<kbd class="rounded border px-1 py-0.5 font-mono text-[10px]">⌘</kbd>
						{t('projects.environment.paint_hint_3')}
						<kbd class="rounded border px-1 py-0.5 font-mono text-[10px]">←↑↓→</kbd>
						{t('projects.environment.paint_hint_4')}
						<kbd class="rounded border px-1 py-0.5 font-mono text-[10px]">Esc</kbd>
						{t('projects.environment.paint_hint_5')}
					{:else}
						{t('projects.environment.tip_hint_1')}
						<kbd class="rounded border px-1 py-0.5 font-mono text-[10px]">Ctrl</kbd>
						{t('projects.environment.paint_hint_2')}
						<kbd class="rounded border px-1 py-0.5 font-mono text-[10px]">⌘</kbd>
						{t('projects.environment.tip_hint_2')}
					{/if}
				</p>
			{/if}

			{#if currentEnvironment.applications.length === 0}
				<div
					class="flex min-h-56 flex-col items-center justify-center rounded-xl border border-dashed p-8 text-center">
					<BoxesIcon class="text-muted-foreground size-8" />
					<h3 class="mt-3 font-medium">
						{t('projects.environment.no_applications', { name: currentEnvironment.name })}
					</h3>
					<p class="mt-1 text-sm text-muted-foreground">
						{t('projects.environment.no_applications_hint')}
					</p>
					<DropdownMenu.Root>
						<DropdownMenu.Trigger
							>{#snippet child({ props })}<Button
									class="mt-4"
									{...props}
									disabled={Boolean(deploymentNotice)}
									><PlusIcon class="size-4" />{t('projects.environment.new_resource')}</Button
								>{/snippet}</DropdownMenu.Trigger>
						<DropdownMenu.Content align="center">
							<DropdownMenu.Item
								onclick={openCreateApplication}
								disabled={Boolean(deploymentNotice)}
								><AppWindowIcon class="size-4" />{t(
									'projects.environment.application',
								)}</DropdownMenu.Item>
							<DropdownMenu.Item
								onclick={openCreateCatalogTemplate}
								disabled={Boolean(deploymentNotice)}
								><BoxesIcon class="size-4" />{t(
									'projects.environment.template',
								)}</DropdownMenu.Item>
							<DropdownMenu.Item onclick={openCreateDatabase} disabled={Boolean(deploymentNotice)}
								><DatabaseIcon class="size-4" />{t(
									'projects.environment.database',
								)}</DropdownMenu.Item>
						</DropdownMenu.Content>
					</DropdownMenu.Root>
				</div>
			{:else}
				{#snippet applicationCard(application: ApplicationDto, appIndex: number)}
					{@const meta = statusMeta[applicationStatus[application.id] ?? '']}
					{@const selected = selectMode && selectedIds.has(application.id)}
					<article
						data-app-index={appIndex}
						class="lp-lifted group relative overflow-hidden rounded-2xl bg-card ring-1 ring-foreground/8 transition-[background-color,box-shadow] hover:bg-accent/30 hover:ring-primary/40"
						class:ring-2={selected}
						class:ring-inset={selected}
						class:ring-primary={selected}>
						<a
							class="absolute inset-0 rounded-2xl outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2"
							class:pointer-events-none={selectMode}
							href={`${environmentApplicationsHref(projectId, currentEnvironment.name)}/${encodeURIComponent(application.id)}`}
							aria-label={t('projects.environment.open_application_aria', {
								name: application.name,
							})}
							tabindex={selectMode ? -1 : 0}></a>
						{#if selectMode}
							<button
								type="button"
								class="absolute inset-0 z-10 rounded-2xl outline-none focus-visible:ring-2 focus-visible:ring-ring"
								aria-pressed={selectedIds.has(application.id)}
								aria-label={t('projects.environment.select_application_named', {
									name: application.name,
								})}
								onclick={() => toggleSelected(application.id)}
								onmouseenter={(event) => paintEnter(event, application.id)}></button>
							<div class="absolute right-3 top-3 z-20 rounded-md bg-card/80 backdrop-blur-sm">
								<SoftCheckbox
									checked={selectedIds.has(application.id)}
									onCheckedChange={(value) => setSelected(application.id, value === true)}
									aria-label={t('projects.environment.select_application_named', {
										name: application.name,
									})} />
							</div>
						{/if}
						<div class="flex items-start gap-3 p-4">
							<div class="bg-muted flex size-10 shrink-0 items-center justify-center rounded-lg">
								<ApplicationIcon
									templateKind={application.template_kind}
									catalogIconUrl={application.catalog_icon_url}
									iconUrl={application.icon_url}
									applicationId={application.id}
									bind:failedApplicationIds={failedIconApplicationIds} />
							</div>
							<div class="min-w-0 flex-1">
								<h3 class="truncate font-semibold">{application.name}</h3>
								<p class="mt-1 line-clamp-2 min-h-10 text-sm text-muted-foreground">
									{application.description || t('projects.projects.no_description')}
								</p>
							</div>
						</div>
						{#if sparklines}
							{@const requests = sparklines.applications[application.id] ?? []}
							{@const hovered =
								hoveredRequests?.id === application.id ? hoveredRequests.index : null}
							<!-- relative: lift above the card's stretched link so the chart gets hover -->
							<div class="relative flex flex-col gap-1.5 px-4 pb-3">
								<CellChart
									values={requests}
									cols={48}
									rows={6}
									tone="muted"
									onhover={(index) =>
										(hoveredRequests = index === null ? null : { id: application.id, index })}
									ariaLabel={t('projects.environment.requests_24h')} />
								<span class="flex items-center justify-between text-[11px] text-muted-foreground">
									{#if hovered !== null}
										<span class="font-mono">{sparklineTime(sparklines, i18n.locale)(hovered)}</span>
										<b class="font-mono font-medium text-foreground tabular-nums"
											>{t('analytics.panel.requests_count', { count: requests[hovered] ?? 0 })}</b>
									{:else}
										{t('projects.environment.requests_24h')}
										<span class="tabular-nums"
											>{compactNumber(
												requests.reduce((a, b) => a + b, 0),
												i18n.locale,
											)}</span>
									{/if}
								</span>
							</div>
						{/if}
						<div class="lp-groove-t flex items-center justify-between gap-2 px-4 py-3">
							<div class="flex min-w-0 items-center gap-2">
								{#if application.server}
									<ServerMark server={application.server} />
								{:else if application.server_group}
									<ServerGroupMark group={application.server_group} />
								{/if}
								{#if meta}
									<span class="flex items-center gap-1.5 text-xs text-muted-foreground">
										<span class={`size-1.5 shrink-0 rounded-full ${meta.dot}`}></span>{meta.label}
									</span>
								{/if}
							</div>
							<div class="relative z-10 flex gap-1">
								<Button
									variant="ghost"
									size="icon"
									aria-label={t('projects.environment.edit_application_named', {
										name: application.name,
									})}
									onclick={() => openEditApplication(application)}
									><PencilIcon class="size-4" /></Button>
								<Button
									variant="ghost"
									size="icon"
									class="text-destructive hover:text-destructive"
									aria-label={t('projects.environment.delete_application_named', {
										name: application.name,
									})}
									onclick={() => confirmDeleteApplication(currentEnvironment, application)}
									><Trash2Icon class="size-4" /></Button>
							</div>
						</div>
					</article>
				{/snippet}

				{#if regularApplications.length > 0}
					<div class="grid gap-4 sm:grid-cols-2 xl:grid-cols-3">
						{#each regularApplications as application, appIndex (application.id)}
							{@render applicationCard(application, appIndex)}
						{/each}
					</div>
				{/if}
				{#if databaseApplications.length > 0}
					<div class="mt-8 flex items-center gap-2">
						<DatabaseIcon class="size-4" />
						<h2 class="text-sm font-semibold tracking-tight">
							{t('projects.environment.databases')}
						</h2>
					</div>
					<div class="mt-3 grid gap-4 sm:grid-cols-2 xl:grid-cols-3">
						{#each databaseApplications as application, appIndex (application.id)}
							{@render applicationCard(application, appIndex)}
						{/each}
					</div>
				{/if}
			{/if}
		</section>
	{/if}
</div>

{#if detail}
	<ProjectFormDialog
		bind:open={projectDialogOpen}
		project={detail.project}
		onsave={handleSaveProject}
		onUploadIcon={handleUploadProjectIcon} />
{/if}
<EnvironmentFormDialog
	bind:open={environmentDialogOpen}
	environment={editingEnvironment}
	onSubmit={handleSaveEnvironment} />
<ApplicationFormDialog
	bind:open={applicationDialogOpen}
	application={editingApplication}
	servers={readyServers}
	{organizationId}
	initialCreateMode={applicationCreateMode}
	onSubmit={handleSaveApplication}
	onUploadIcon={handleUploadApplicationIcon}
	onCreateFromTemplate={handleCreateCatalogTemplate} />
{#if organizationId && environment}
	<DatabaseTemplateDialog
		bind:open={databaseTemplateDialogOpen}
		{organizationId}
		{projectId}
		environmentId={environment.id}
		servers={readyServers}
		onPostgresCreated={handlePostgresCreated}
		onMariaDbCreated={handleMariaDbCreated}
		onDragonflyDbCreated={handleDragonflyDbCreated}
		onKeyDbCreated={handleKeyDbCreated}
		onSqlServerCreated={handleSqlServerCreated}
		onMongoDbCreated={handleMongoDbCreated} />
{/if}
<DeleteResourceDialog
	bind:open={deleteDialogOpen}
	resourceName={deleteTarget?.name ?? ''}
	resourceType={deleteTarget?.type ?? 'project'}
	cascadeDescription={deleteCascadeDescription()}
	ondelete={handleDelete} />
<DeleteResourceDialog
	bind:open={bulkDeleteDialogOpen}
	resourceName={t(
		`projects.environment.bulk_delete_name_${selectedIds.size === 1 ? 'one' : 'many'}`,
		{ count: selectedIds.size },
	)}
	resourceType="application"
	cascadeDescription={t('projects.environment.bulk_cascade')}
	ondelete={handleBulkDelete} />

{#if organizationId && accessOpen}
	<Dialog.Root bind:open={accessOpen}>
		<Dialog.Content class="flex flex-col gap-0 overflow-y-hidden p-0 sm:max-w-2xl">
			<Dialog.Header class="border-b p-6 pr-14">
				<Dialog.Title>{t('organizations.access.access')}</Dialog.Title>
				<Dialog.Description>{t('organizations.access.dialog_hint')}</Dialog.Description>
			</Dialog.Header>
			<div class="min-h-0 flex-1 overflow-y-auto p-6 space-y-8">
				<AccessPanel {organizationId} scopeType="project" scopeId={projectId} />
				{#if environment}
					{#key environment.id}
						<AccessPanel
							{organizationId}
							scopeType="environment"
							scopeId={environment.id}
							{projectId} />
					{/key}
				{/if}
			</div>
		</Dialog.Content>
	</Dialog.Root>
{/if}
