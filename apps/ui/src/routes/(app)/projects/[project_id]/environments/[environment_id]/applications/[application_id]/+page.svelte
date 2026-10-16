<script lang="ts">
	import type { DomainInput } from '#lib/clients/project-client.js';
	import { onDestroy, onMount } from 'svelte';
	import { page } from '$app/state';
	import { afterNavigate, goto, refreshAll } from '$app/navigation';
	import SkeletonRows from '#lib/components/custom/skeleton-rows.svelte';
	import ApplicationGeneralTab from '#lib/components/projects/application-general-tab.svelte';
	import ApplicationEnvironmentTab from '#lib/components/projects/application-environment-tab.svelte';
	import ApplicationContainersTab from '#lib/components/projects/application-containers-tab.svelte';
	import ApplicationNetworksTab from '#lib/components/projects/application-networks-tab.svelte';
	import ApplicationMapTab from '#lib/components/projects/application-map-tab.svelte';
	import ApplicationVolumesTab from '#lib/components/projects/application-volumes-tab.svelte';
	import ApplicationDomainsTab from '#lib/components/projects/application-domains-tab.svelte';
	import PostgresResourceDetail from '#lib/components/projects/postgres-resource-detail.svelte';
	import MariaDbResourceDetail from '#lib/components/projects/mariadb-resource-detail.svelte';
	import DragonflyDbResourceDetail from '#lib/components/projects/dragonflydb-resource-detail.svelte';
	import KeyDbResourceDetail from '#lib/components/projects/keydb-resource-detail.svelte';
	import SqlServerResourceDetail from '#lib/components/projects/sqlserver-resource-detail.svelte';
	import ServerMark from '#lib/components/servers/server-mark.svelte';
	import ServerGroupMark from '#lib/components/server-groups/server-group-mark.svelte';
	import { getServerGroup } from '#lib/clients/server-group-client.js';
	import ApplicationIcon from '#lib/components/projects/application-icon.svelte';
	import ProjectIcon from '#lib/components/projects/project-icon.svelte';
	import { isDisplayableImageUrl } from '#lib/utils/application-card-icon.js';
	import MongoDbResourceDetail from '#lib/components/projects/mongodb-resource-detail.svelte';
	import RedeployConfirmDialog from '#lib/components/projects/redeploy-confirm-dialog.svelte';
	import RedeployWithPullDialog from '#lib/components/projects/redeploy-with-pull-dialog.svelte';
	import ApplicationLogsTab from '#lib/components/projects/application-logs-tab.svelte';
	import { askAboutLogs } from '#lib/components/assistant/thread.svelte.js';
	import ApplicationDeploymentTab from '#lib/components/projects/application-deployment-tab.svelte';
	import ApplicationTerminalTab from '#lib/components/projects/application-terminal-tab.svelte';
	import ApplicationMetricsTab from '#lib/components/projects/application-metrics-tab.svelte';
	import AnalyticsPanel from '#lib/components/analytics/analytics-panel.svelte';
	import { getApplicationAnalytics } from '#lib/clients/analytics-client.js';
	import ApplicationTasksTab from '#lib/components/projects/application-tasks-tab.svelte';
	import ApplicationAdvancedTab from '#lib/components/projects/application-advanced-tab.svelte';
	import ApplicationBootPolicyCard from '#lib/components/projects/application-boot-policy-card.svelte';
	import DatabaseBackupTab from '#lib/components/projects/database-backup-tab.svelte';
	import ApplicationFormDialog from '#lib/components/projects/application-form-dialog.svelte';
	import DeleteResourceDialog from '#lib/components/projects/delete-resource-dialog.svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Tooltip from '#lib/components/ui/tooltip/index.js';
	import { tabIndexFromKeydown } from '#lib/utils/tab-shortcut.js';
	import PencilIcon from '@lucide/svelte/icons/pencil';
	import ServerIcon from '@lucide/svelte/icons/server';
	import BoxesIcon from '@lucide/svelte/icons/boxes';
	import FolderKanbanIcon from '@lucide/svelte/icons/folder-kanban';
	import LayersIcon from '@lucide/svelte/icons/layers';
	import Trash2Icon from '@lucide/svelte/icons/trash-2';
	import {
		applicationVolumeWorkspace,
		managedVolumeWorkspace,
		composeContentGet,
		composeContentPut,
		composeDeploy,
		composeStart as composeStartApi,
		composeStatus as composeStatusApi,
		composeStop as composeStopApi,
		composeValidate,
		createDomain,
		deleteApplication,
		deleteDeploymentRun,
		deleteDeploymentRuns,
		deleteDomain,
		deployApplication,
		retryReplicaDeploy,
		generateDomain,
		getApplication,
		getResourceBaseline,
		getApplicationSource,
		getApplicationVolumes,
		listManagedVolumes,
		createManagedVolume,
		attachManagedVolume,
		detachManagedVolume,
		deleteManagedVolume,
		getPortConstraints,
		getProject,
		getDeployment,
		getEnvironmentVariables,
		getDeploymentRun,
		listDeploymentQueue,
		listDeploymentRuns,
		listDomains,
		validateDomainDns,
		validateHostDns,
		saveApplicationSource,
		saveEnvironmentVariables,
		setCleanBuild,
		stopApplication,
		startApplication,
		removeApplicationRuntime,
		streamDeploymentQueueEvents,
		updateDomain,
		uploadApplicationArtifact,
		deleteApplicationArtifact,
		updateApplication,
		uploadApplicationIcon,
		getContainerStatus,
		getApplicationNetworks,
		listResourceNetworks,
		attachResourceNetwork,
		detachResourceNetwork,
		probeApplicationNetwork,
		getApplicationNetworkStats,
		randomSslipHost,
		listComposeProxyRoutes,
		saveComposeProxyRoute,
		getDomainMarkers,
		getBasicAuth,
		updateBasicAuth,
		addBasicAuthUser,
		deleteBasicAuthUser,
		type ComposeProxyRoute,
		type DomainMarkersDto,
		type ApplicationDto,
		type BasicAuthConfigDto,
		type ComposeServiceStatus,
		type ContainerStatusDto,
		type ApplicationNetworkOverviewDto,
		type ResourceNetworkLinkDto,
		type LinkedApplicationDto,
		type DeploymentDto,
		type DeploymentJobDto,
		type DeploymentQueueEvent,
		type DeploymentRunDetailDto,
		type DeploymentRunDto,
		type DeploymentRunLogDto,
		type DomainDto,
		type EnvironmentDto,
		listProjects,
		type PortConstraintsDto,
		type PortMapping,
		type ProjectDetailDto,
		type ProjectDto,
		type ServerSummaryDto,
		type SourceConfigDto,
		type ValidationResponse,
		type VolumeDto,
		type ManagedVolumeDto,
		type CreateManagedVolumeInput,
		getDeployHook,
		rotateDeployHook,
		rollbackDeploymentRun,
		type DeployHookDto,
		getHealthCheck,
		updateHealthCheck,
		type HealthCheckDto,
		type UpdateHealthCheckInput,
	} from '#lib/clients/project-client.js';
	import {
		listGitBranches,
		listPublicGitBranches,
		listGitProviders,
		listGitRepositories,
		type GitProviderDto,
		type GitRepositoryDto,
	} from '#lib/clients/git-provider-client.js';
	import { getServer, type Server } from '#lib/clients/server-client.js';
	import { listMembers, listOrganizations } from '#lib/clients/organization-client.js';
	import { getMe } from '#lib/clients/auth-client.js';
	import ServerTerminalDialog from '#lib/components/servers/server-terminal-dialog.svelte';
	import { catalogTemplateDocsUrl, getCatalogTemplate } from '#lib/clients/template-client.js';
	import {
		resolveSelectedOrganization,
		setSelectedOrganizationId,
	} from '#lib/utils/selected-organization.js';
	import { setPageBreadcrumbs } from '#lib/state/page-header.svelte.js';
	import {
		clearScopeHero,
		clearShellTabs,
		setScopeHero,
		setShellTabs,
	} from '#lib/state/shell.svelte.js';
	import { APPLICATION_TAB_ICONS } from '#lib/navigation/application-tab-icons.js';
	import RefreshCwIcon from '@lucide/svelte/icons/refresh-cw';
	import PlayIcon from '@lucide/svelte/icons/play';
	import SquareIcon from '@lucide/svelte/icons/square';
	import {
		createDeferredTabLoader,
		type ApplicationDeferredTab,
	} from '#lib/utils/application-tab-loaders.js';
	import { environmentApplicationsHref, resolveEnvironment } from '#lib/utils/environment-route.js';
	import { queueEventAction } from '#lib/utils/deployment-queue-events.js';
	import { toast } from '#lib/utils/toast.js';
	import { invalidateOnboarding } from '#lib/state/onboarding.svelte.js';
	import { allowed } from '#lib/state/permissions.svelte.js';
	import { isRunning } from '#lib/utils/container-status.js';
	import { makePrivateNetworkExplicit } from '#lib/utils/compose-private-network.js';
	import {
		metricContainerIds,
		metricIndex,
		streamApplicationContainerMetrics,
		type ContainerMetric,
		type ContainerMetricsSnapshot,
	} from '#lib/clients/container-metrics-client.js';
	import { t } from '#lib/i18n/index.svelte.js';

	const projectId = $derived(page.params.project_id ?? '');
	const environmentReference = $derived(page.params.environment_id ?? '');
	const applicationId = $derived(page.params.application_id ?? '');
	let projects = $state<ProjectDto[]>([]);
	let projectDetail = $state<ProjectDetailDto | null>(null);
	let environment = $state<EnvironmentDto | null>(null);
	let application = $state<ApplicationDto | null>(null);
	// Catalog-provided `ui` path, resolved by the API. Falls back to the derived
	// path while it loads or when the catalog is unreachable.
	let templateUiUrl = $state<string | null>(null);

	const templateDocsUrl = $derived(
		application?.catalog_template_id
			? (templateUiUrl ?? catalogTemplateDocsUrl(application.catalog_template_id))
			: null,
	);

	$effect(() => {
		const templateId = application?.catalog_template_id;
		if (!templateId) {
			templateUiUrl = null;
			return;
		}
		let active = true;
		void getCatalogTemplate(templateId)
			.then((template) => {
				if (active) templateUiUrl = template.ui_url;
			})
			.catch(() => {});
		return () => {
			active = false;
		};
	});
	let loading = $state(true);
	let error = $state<string | null>(null);
	let failedIconApplicationIds = $state<Set<string>>(new Set());
	let projectIconFailed = $state(false);
	$effect(() => {
		// Reset the fallback when the project (or its icon) changes.
		void projectDetail?.project.icon_url;
		projectIconFailed = false;
	});
	let organizationId = $state('');
	let generation = 0;
	let deployment = $state<DeploymentDto | null>(null);
	let runtimeEnvironmentContent = $state('');
	let buildEnvironmentContent = $state('');
	// Names only, so the Compose editor can flag undefined ${VAR} without waiting
	// for the deferred Environment tab to load.
	let runtimeVariableNames = $state<string[]>([]);
	let busy = $state(false);
	let volumes = $state<VolumeDto[]>([]);
	let managedVolumes = $state<ManagedVolumeDto[]>([]);
	let domains = $state<DomainDto[]>([]);
	let domainServer = $state<Server | null>(null);
	// Host shell row on the Terminals tab: same gate as the servers page console
	// (SysAdmin or org Admin), and only for single-server apps; a group app has no
	// one host to open, so the row stays hidden.
	let hostServer = $state<Server | null>(null);
	let hostShellOpen = $state(false);
	let hostServerLoadedFor = '';
	$effect(() => {
		const serverId = application?.server?.id;
		if (activeTab !== 'terminal' || !serverId || !organizationId) return;
		const key = organizationId + '/' + serverId;
		if (hostServerLoadedFor === key) return;
		hostServerLoadedFor = key;
		hostServer = null;
		const orgId = organizationId;
		void (async () => {
			const me = await getMe();
			const canManage =
				me.is_sysadmin ||
				(await listMembers(orgId)).some(
					(member) => member.user_id === me.user_id && member.role_id <= 1,
				);
			if (!canManage) return;
			const server = await getServer(orgId, serverId);
			if (hostServerLoadedFor === key && server.state === 'ready') hostServer = server;
		})().catch(() => {});
	});
	let pinServers = $state<ServerSummaryDto[]>([]);
	let source = $state<SourceConfigDto | null>(null);
	let basicAuth = $state<BasicAuthConfigDto | null>(null);
	let gitProviders = $state<GitProviderDto[]>([]);
	let deployHook = $state<DeployHookDto | null>(null);
	let healthCheck = $state<HealthCheckDto | null>(null);
	let portConstraints = $state<PortConstraintsDto>({ reserved_ports: [], taken_ports: [] });
	let redeployOpen = $state(false);
	let redeployPort = $state<number | null>(null);
	let redeployWithPullOpen = $state(false);
	let editApplicationOpen = $state(false);
	let deleteDialogOpen = $state(false);
	let deploymentRuns = $state<DeploymentRunDto[]>([]);
	let selectedDeploymentRun = $state<DeploymentRunDetailDto | null>(null);
	let liveLogsPaused = $state(false);
	let activeDeploymentJobId = $state<string | null>(null);
	let activeDeploymentStatus = $state<string | null>(null);
	const ACTIVE_JOB_STATUSES = ['queued', 'building', 'deploying'];
	// The run a live SSE stream is actually reporting on. `liveStatus` must
	// only reach the deployment tab when this matches the selected run -
	// otherwise selecting an old run shows whatever the live job's status
	// happens to be, not that run's own status.
	let activeDeploymentRunId = $state<string | null>(null);
	const MAX_VISIBLE_DEPLOYMENT_LOGS = 1_000;
	let deploymentStreamState = $state<'idle' | 'live' | 'reconnecting' | 'error'>('idle');
	let deploymentStreamRetryCount = $state(0);
	let deploymentEvents: EventSource | null = null;
	let containerStatus = $state<ContainerStatusDto | null>(null);
	let networkOverview = $state<ApplicationNetworkOverviewDto | null>(null);
	let networkError = $state<string | null>(null);
	let resourceLinks = $state<ResourceNetworkLinkDto[]>([]);
	let resourceConsumers = $state<LinkedApplicationDto[]>([]);
	let resourceLinksLoading = $state(false);
	let containerMetrics = $state<Map<string, ContainerMetric>>(new Map());
	let composeContent = $state('');
	let composePortMappings = $state<PortMapping[]>([]);
	let composeServiceStatuses = $state<ComposeServiceStatus[]>([]);
	let composeServices = $state<{ name: string; suggested_internal_ports: number[] }[]>([]);
	let composeProxyRoutes = $state<ComposeProxyRoute[]>([]);
	let domainMarkers = $state<DomainMarkersDto | null>(null);
	let refreshingTab = $state(false);
	// Runtime status is not cosmetic outside General: it gates the Terminal and
	// Containers tabs and the start/stop controls, so it keeps refreshing everywhere,
	// just more slowly where it is not on screen.
	const CONTAINER_STATUS_POLL_ACTIVE_MS = 15_000;
	const CONTAINER_STATUS_POLL_BACKGROUND_MS = 60_000;
	const STATUS_VISIBLE_TABS = ['general', 'containers', 'logs', 'terminal'];
	const baseTabItems = [
		{ value: 'general', label: t('projects.application.general') },
		{ value: 'environment', label: t('projects.application.environment') },
		{ value: 'domains', label: t('projects.application.domains') },
		{ value: 'deployment', label: t('projects.application.deployment') },
		{ value: 'containers', label: t('projects.application.containers') },
		{ value: 'volumes', label: t('projects.application.volumes') },
		{ value: 'terminal', label: t('projects.application.terminal') },
		{ value: 'logs', label: t('projects.application.logs') },
		{ value: 'metrics', label: t('projects.application.metrics') },
		{ value: 'analytics', label: t('projects.application.analytics') },
		{ value: 'tasks', label: t('projects.application.tasks') },
		{ value: 'advanced', label: t('projects.application.advanced') },
	];
	const defaultTab = baseTabItems[0].value;
	const isInfrastructureTemplate = $derived(
		application?.template_kind === 'postgres' ||
			application?.template_kind === 'mariadb' ||
			application?.template_kind === 'dragonflydb' ||
			application?.template_kind === 'keydb' ||
			application?.template_kind === 'sqlserver' ||
			application?.template_kind === 'mongodb',
	);
	// Managed databases with a backup path (SQL Server's .bak flow is excluded).
	const isDatabaseBackupSupported = $derived(
		['postgres', 'mariadb', 'mongodb', 'dragonflydb', 'keydb'].includes(
			application?.template_kind ?? '',
		),
	);
	const hasRunningContainer = $derived(
		source?.deploy_type === 'compose'
			? composeServiceStatuses.some((service) => isRunning(service.status))
			: isRunning(containerStatus?.status ?? ''),
	);
	// Networks and Map live inside the Containers tab now. With nothing deployed
	// all three sections render their own "nothing here" box, which stacks into
	// three consecutive empty states; the container one already says what to do.
	const hasNetworkData = $derived(
		(networkOverview?.networks.length ?? 0) > 0 || (networkOverview?.containers.length ?? 0) > 0,
	);
	// Status by container name for the Map tab's LEDs (the /networks payload has no status).
	const mapStatuses = $derived(
		source?.deploy_type === 'compose'
			? Object.fromEntries(composeServiceStatuses.map((s) => [s.container_name, s.status]))
			: containerStatus?.container_name
				? { [containerStatus.container_name]: containerStatus.status }
				: {},
	);
	function buildTabItems() {
		const target = { projectId, environmentId: environment?.id };
		const items = baseTabItems
			// Databases serve no HTTP: no domains, no request analytics.
			.filter(
				(item) =>
					!isInfrastructureTemplate || (item.value !== 'domains' && item.value !== 'analytics'),
			)
			.filter((item) => item.value !== 'terminal' || allowed('app.exec', target))
			.map((item) =>
				isInfrastructureTemplate && item.value === 'general'
					? { ...item, label: t('projects.application.overview') }
					: item,
			)
			.map((item) => {
				if (item.value === 'terminal') {
					return {
						...item,
						disabled: !hasRunningContainer,
						disabledTooltip: t('projects.application.terminal_disabled'),
					};
				}
				return item;
			});
		if (isDatabaseBackupSupported && allowed('backup.read', target)) {
			items.splice(1, 0, { value: 'backups', label: t('projects.application.backups') });
		}
		return items;
	}
	const tabItems = $derived(buildTabItems());

	// Tab selection mirrors ?tab= so it survives reload and deep links. Local
	// state stays the source of truth for rendering; the URL follows it.
	// Networks and Map were folded into Containers; keep their links working.
	const MERGED_TABS: Record<string, string> = { networks: 'containers', map: 'containers' };

	function tabFromUrl(url: typeof page.url) {
		const requested = url.searchParams.get('tab');
		if (!requested) return defaultTab;
		const resolved = MERGED_TABS[requested] ?? requested;
		return tabItems.some((item) => item.value === resolved) ? resolved : defaultTab;
	}

	let activeTab = $state(tabFromUrl(page.url));

	// Back/forward navigation changes the URL without going through selectTab.
	// Scoped to popstate only — reacting to page.url directly would also fire
	// on our own replaceState() from selectTab and stomp the tab it just set.
	afterNavigate((navigation) => {
		if (navigation.shallow) return;
		if (navigation.type !== 'popstate') return;
		activeTab = tabFromUrl(page.url);
		void loadDeferredTab(activeTab);
	});

	async function refreshRuntimeStatus(deployType = source?.deploy_type) {
		if (!organizationId || !environment) return;
		if (healthCheck?.enabled) {
			getHealthCheck(organizationId, projectId, environment.id, applicationId)
				.then((next) => (healthCheck = next))
				.catch(() => {
					// Keep the last known health status when a transient check fails.
				});
		}
		if (deployType === 'compose')
			composeServiceStatuses = await composeStatusApi(
				organizationId,
				projectId,
				environment.id,
				applicationId,
			);
		else
			containerStatus = await getContainerStatus(
				organizationId,
				projectId,
				environment.id,
				applicationId,
			);
	}

	// Poll on every tab, faster where the status is actually rendered. A hidden browser
	// tab skips its ticks and refreshes on the way back: the API may have restarted, or
	// the container may have died, while it sat in the background.
	$effect(() => {
		const deployType = source?.deploy_type;
		if (!organizationId || !environment || loading) return;
		const refresh = () =>
			void refreshRuntimeStatus(deployType).catch(() => {
				// Keep the last known status when a transient runtime check fails.
			});
		refresh();
		const interval = setInterval(
			() => {
				if (document.hidden) return;
				refresh();
			},
			STATUS_VISIBLE_TABS.includes(activeTab)
				? CONTAINER_STATUS_POLL_ACTIVE_MS
				: CONTAINER_STATUS_POLL_BACKGROUND_MS,
		);
		const onVisibilityChange = () => {
			if (!document.hidden) refresh();
		};
		document.addEventListener('visibilitychange', onVisibilityChange);
		return () => {
			clearInterval(interval);
			document.removeEventListener('visibilitychange', onVisibilityChange);
		};
	});

	let metricsContainerNames = $derived(
		metricContainerIds(
			containerStatus,
			source?.deploy_type === 'compose' ? composeServiceStatuses : [],
		),
	);
	$effect(() => {
		if (
			(activeTab !== 'general' && activeTab !== 'containers') ||
			metricsContainerNames.length === 0 ||
			!organizationId ||
			!environment
		) {
			containerMetrics = new Map();
			return;
		}
		const stream = streamApplicationContainerMetrics(
			organizationId,
			projectId,
			environment.id,
			applicationId,
		);
		const updateMetrics = (event: Event) => {
			try {
				containerMetrics = metricIndex(
					JSON.parse((event as MessageEvent<string>).data) as ContainerMetricsSnapshot,
				);
			} catch {
				// Ignore malformed stream events and preserve the last valid metrics.
			}
		};
		stream.addEventListener('metrics', updateMetrics);
		return () => {
			stream.removeEventListener('metrics', updateMetrics);
			stream.close();
		};
	});

	function isDeferredTab(value: string): value is ApplicationDeferredTab {
		return [
			'containers',
			'deployment',
			'domains',
			'environment',
			'networks',
			'map',
			'volumes',
		].includes(value);
	}

	async function refreshNetworks() {
		if (!environment || !organizationId) return;
		networkError = null;
		void refreshResourceLinks();
		try {
			networkOverview = await getApplicationNetworks(
				organizationId,
				projectId,
				environment.id,
				applicationId,
			);
		} catch (caught) {
			networkError = caught instanceof Error ? caught.message : 'Could not load network topology';
			throw caught;
		}
	}

	// Linkable resources come from the database, not from Podman, so a failure
	// here must not blank out the topology the tab is really about.
	async function refreshResourceLinks() {
		if (!environment || !organizationId) return;
		resourceLinksLoading = true;
		try {
			({ linkable: resourceLinks, consumers: resourceConsumers } = await listResourceNetworks(
				organizationId,
				projectId,
				environment.id,
				applicationId,
			));
		} catch (caught) {
			toast.error(
				caught instanceof Error ? caught.message : 'Could not load the database resources',
			);
		} finally {
			resourceLinksLoading = false;
		}
	}

	async function toggleResourceLink(resourceId: string, linked: boolean) {
		if (!environment || !organizationId) return;
		try {
			({ linkable: resourceLinks, consumers: resourceConsumers } = linked
				? await attachResourceNetwork(
						organizationId,
						projectId,
						environment.id,
						applicationId,
						resourceId,
					)
				: await detachResourceNetwork(
						organizationId,
						projectId,
						environment.id,
						applicationId,
						resourceId,
					));
			toast.success('Redeploy the application to apply the network change.');
		} catch (caught) {
			toast.error(caught instanceof Error ? caught.message : 'Could not change the network link');
		}
	}

	async function probeNetworks() {
		if (!environment || !organizationId) return [];
		const result = await probeApplicationNetwork(
			organizationId,
			projectId,
			environment.id,
			applicationId,
		);
		return result.pairs;
	}

	async function statsNetworks() {
		if (!environment || !organizationId) return [];
		const result = await getApplicationNetworkStats(
			organizationId,
			projectId,
			environment.id,
			applicationId,
		);
		return result.containers;
	}

	const deferredTabs = createDeferredTabLoader({
		// Containers renders the container list, the network overview and the
		// topology map, so opening it has to fetch both sides.
		containers: async () => {
			await Promise.all([refreshRuntimeStatus(source?.deploy_type), refreshNetworks()]);
		},
		deployment: refreshDeploymentRuns,
		domains: async () => {
			await refreshDomains();
			if (application?.server?.kind === 'ssh') {
				domainServer = await getServer(organizationId, application.server.id).catch(() => null);
			} else domainServer = null;
			if (source?.deploy_type === 'compose') {
				composeProxyRoutes = await listComposeProxyRoutes(
					organizationId,
					projectId,
					environment!.id,
					applicationId,
				);
			}
		},
		environment: refreshEnvironmentVariables,
		networks: refreshNetworks,
		map: refreshNetworks,
		volumes: refreshVolumeState,
	});

	async function loadDeferredTab(value: string) {
		if (!isDeferredTab(value) || !environment || !organizationId || loading) return;
		refreshingTab = true;
		try {
			if (value === 'deployment') await deferredTabs.refresh(value);
			else await deferredTabs.load(value);
		} catch (caught) {
			toast.error(caught instanceof Error ? caught.message : `Could not load ${value}`);
		} finally {
			refreshingTab = false;
		}
	}

	function selectTab(value: string) {
		activeTab = value;
		const url = new URL(page.url.href);

		if (value === defaultTab) url.searchParams.delete('tab');
		else url.searchParams.set('tab', value);

		goto(url, { shallow: true, replace: true, state: page.state });

		if (isDeferredTab(value)) void loadDeferredTab(value);
		else void refreshTab(value);
	}

	function makePrivateNetworkEditable() {
		const result = makePrivateNetworkExplicit(composeContent, applicationId);
		if (result.kind === 'updated') {
			composeContent = result.content;
			selectTab('general');
			toast.success('Private network added to Compose. Review, save, and redeploy to apply it.');
			return;
		}
		toast.info(
			result.kind === 'already-configured'
				? 'This Compose file already has network configuration. Edit it directly.'
				: 'Could not safely add a network. Edit the Compose file directly.',
		);
	}

	function showDeploymentFailure() {
		toast.error('Deployment failed', {
			action: { label: 'View deployment', onClick: () => selectTab('deployment') },
		});
	}

	function handleTabShortcut(event: KeyboardEvent) {
		// Scoped to the active group, so the shortcut never runs past digit 9.
		const index = tabIndexFromKeydown(event, tabItems.length);
		if (index === null) return;
		const target = tabItems[index];
		if ('disabled' in target && target.disabled) return;
		event.preventDefault();
		selectTab(target.value);
	}

	function switchProject(newProjectId: string) {
		void goto(`/projects/${encodeURIComponent(newProjectId)}`);
	}

	function switchEnvironment(newEnvironmentId: string) {
		const target = projectDetail?.environments.find((item) => item.id === newEnvironmentId);
		if (!target || target.id === environment?.id) return;
		void goto(
			`/projects/${encodeURIComponent(projectId)}/environments/${encodeURIComponent(target.name)}`,
		);
	}

	function switchApplication(newApplicationId: string) {
		if (newApplicationId === applicationId) return;
		void goto(
			`${environmentApplicationsHref(projectId, environment?.name ?? environmentReference)}/${encodeURIComponent(newApplicationId)}`,
		);
	}

	async function loadApplication(pId: string, ref: string, appId: string) {
		const gen = ++generation;
		loading = true;
		error = null;
		projectDetail = null;
		environment = null;
		application = null;
		deployment = null;
		source = null;
		deployHook = null;
		containerStatus = null;
		networkOverview = null;
		networkError = null;
		resourceLinks = [];
		resourceConsumers = [];
		gitProviders = [];
		domains = [];
		domainServer = null;
		portConstraints = { reserved_ports: [], taken_ports: [] };
		deploymentRuns = [];
		selectedDeploymentRun = null;
		managedVolumes = [];
		volumes = [];
		runtimeEnvironmentContent = '';
		buildEnvironmentContent = '';
		runtimeVariableNames = [];
		composeContent = '';
		composePortMappings = [];
		composeServiceStatuses = [];
		composeServices = [];
		composeProxyRoutes = [];
		domainMarkers = null;
		deferredTabs.reset();
		try {
			const organization = resolveSelectedOrganization(await listOrganizations());
			if (gen !== generation) return;
			if (!organization) {
				error = 'Choose an organization before viewing an application.';
				return;
			}
			organizationId = organization.id;
			setSelectedOrganizationId(organization.id);
			if (gen !== generation) return;
			const [detail, projectList] = await Promise.all([
				getProject(organizationId, pId),
				listProjects(organizationId),
			]);
			if (gen !== generation) return;
			projectDetail = detail;
			projects = projectList;
			const resolvedEnvironment = resolveEnvironment(detail, ref);
			if (!resolvedEnvironment) {
				error = 'Environment not found.';
				return;
			}
			environment = resolvedEnvironment;
			if (gen !== generation) return;

			const [
				app,
				nextDeployment,
				nextSource,
				nextContainerStatus,
				nextDeployHook,
				nextHealthCheck,
				providers,
			] = await Promise.all([
				getApplication(organizationId, pId, resolvedEnvironment.id, appId),
				getDeployment(organizationId, pId, resolvedEnvironment.id, appId),
				getApplicationSource(organizationId, pId, resolvedEnvironment.id, appId),
				getContainerStatus(organizationId, pId, resolvedEnvironment.id, appId),
				getDeployHook(organizationId, pId, resolvedEnvironment.id, appId).catch(() => null),
				getHealthCheck(organizationId, pId, resolvedEnvironment.id, appId).catch(() => null),
				listGitProviders(organizationId),
			]);
			if (gen !== generation) return;
			application = app;
			pinServers = app.server_group
				? await getServerGroup(organizationId, app.server_group.id)
						.then((group) => group.member_servers)
						.catch(() => [])
				: [];
			if (
				['postgres', 'mariadb', 'dragonflydb', 'keydb', 'sqlserver', 'mongodb'].includes(
					app.template_kind ?? '',
				) &&
				activeTab === 'domains'
			) {
				activeTab = defaultTab;
				const url = new URL(page.url.href);
				url.searchParams.delete('tab');
				goto(url, { shallow: true, replace: true, state: page.state });
			}
			deployment = nextDeployment;
			domains = nextDeployment.domains;
			source = nextSource;
			containerStatus = nextContainerStatus;
			deployHook = nextDeployHook;
			healthCheck = nextHealthCheck;
			gitProviders = providers;
			if (nextSource.deploy_type === 'compose') {
				const [contentResult, statusResult, runtimeNamesResult] = await Promise.allSettled([
					composeContentGet(organizationId, pId, resolvedEnvironment.id, appId),
					composeStatusApi(organizationId, pId, resolvedEnvironment.id, appId),
					getEnvironmentVariables(organizationId, pId, resolvedEnvironment.id, appId, 'runtime'),
				]);
				if (gen !== generation) return;
				if (contentResult.status === 'fulfilled') {
					const data = contentResult.value;
					composeContent = data.content;
					composePortMappings = data.port_mappings;
					composeServices = data.catalog?.services ?? [];
				}
				if (statusResult.status === 'fulfilled') {
					composeServiceStatuses = statusResult.value;
				}
				if (runtimeNamesResult.status === 'fulfilled') {
					runtimeVariableNames = runtimeNamesResult.value.map((variable) => variable.name);
				}
			}
			if (
				nextSource.deploy_type === 'git' &&
				nextSource.build_method === 'compose' &&
				nextSource.compose_public_service
			) {
				composeServices = [
					{ name: nextSource.compose_public_service, suggested_internal_ports: [] },
				];
			}
			setPageBreadcrumbs([
				{ label: t('projects.common.projects'), href: '/projects', icon: FolderKanbanIcon },
				{
					label: detail.project.name,
					iconSnippet: projectCrumbIcon,
					switcher: {
						items: projectList.map((item) => ({
							value: item.id,
							label: item.name,
							description: item.description || undefined,
							icon: item.id,
						})),
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
						items: detail.environments.map((item) => ({
							value: item.id,
							label: item.name,
							description: item.description || (item.is_production ? 'prod' : undefined),
						})),
						selectedValue: resolvedEnvironment.id,
						onSelect: switchEnvironment,
						label: t('projects.common.select_environment'),
						searchPlaceholder: t('projects.common.search_environments'),
						emptyText: t('projects.common.no_environment_found'),
					},
				},
				{
					label: app.name,
					iconSnippet: appCrumbIcon,
					switcher: {
						items: resolvedEnvironment.applications.map((item) => ({
							value: item.id,
							label: item.name,
							description: item.template_kind ?? undefined,
							icon: item.id,
						})),
						selectedValue: appId,
						onSelect: switchApplication,
						itemIcon: appItemIcon,
						label: t('projects.application.select_application'),
						searchPlaceholder: t('projects.application.search_applications'),
						emptyText: t('projects.application.no_application_found'),
					},
				},
			]);
			// A deploy started elsewhere (or before a reload) is still running, so
			// reconnect to its persisted history and live queue events.
			if (nextDeployment.status === 'deploying') void resumeLiveDeployment();
		} catch (caught) {
			if (gen !== generation) return;
			error = caught instanceof Error ? caught.message : 'Failed to load application';
		} finally {
			if (gen === generation) {
				loading = false;
				void loadDeferredTab(activeTab);
				if (activeTab === 'advanced') void refreshTab(activeTab);
			}
		}
	}

	$effect(() => {
		const pId = projectId;
		const ref = environmentReference;
		const appId = applicationId;
		if (!pId || !ref || !appId) return;
		stopDeploymentEvents();
		loadApplication(pId, ref, appId);
	});

	onDestroy(() => {
		++generation;
		stopDeploymentEvents();
		clearScopeHero();
		clearShellTabs();
	});

	// The shell draws the info card and the tab capsule; this page only says what goes in them.
	$effect(() => {
		if (!application || !environment || !projectDetail) return;
		const current = application;
		const base = `/projects/${encodeURIComponent(projectId)}/environments/${encodeURIComponent(environment.id)}/applications`;
		const compose = source?.deploy_type === 'compose';
		const total = compose ? composeServiceStatuses.length : containerStatus ? 1 : 0;
		const running = compose
			? composeServiceStatuses.filter((service) => isRunning(service.status)).length
			: hasRunningContainer
				? 1
				: 0;
		const primary = domains.find((domain) => domain.is_primary) ?? domains[0];
		setScopeHero({
			title: current.name,
			avatar: heroAvatar,
			status: heroStatus,
			meta: heroMeta,
			actions: heroActions,
			stats: [
				{ label: t('projects.application.containers'), value: `${running}/${total}` },
				{
					label: t('projects.application.domains'),
					value: primary
						? domains.length > 1
							? `${primary.host} +${domains.length - 1}`
							: primary.host
						: '-',
				},
				{
					label: t('projects.application_form.server'),
					value: current.server?.name ?? current.server_group?.name ?? '-',
				},
				{ label: t('projects.application_form.source'), value: source?.deploy_type ?? '-' },
			],
			siblingsLabel: t('projects.application.apps_in', { project: projectDetail.project.name }),
			siblings: environment.applications.map((item) => ({
				id: item.id,
				label: item.name,
				href: `${base}/${encodeURIComponent(item.id)}`,
				current: item.id === current.id,
			})),
		});
	});

	$effect(() => {
		if (!deployment) return;
		setShellTabs(
			tabItems.map((item) => ({ ...item, icon: APPLICATION_TAB_ICONS[item.value] })),
			activeTab,
			selectTab,
		);
	});

	// The assistant writes compose files / env vars / source updates and then
	// fires this event from the panel. Any other page already on this
	// application would otherwise keep showing the old buffer; invalidate the
	// route's load so the editor reloads without an F5.
	onMount(() => {
		const handler = (event: Event) => {
			const detail = (event as CustomEvent<{ applicationId?: string }>).detail;
			if (detail?.applicationId !== applicationId) return;
			void refreshAll();
		};
		window.addEventListener('application-updated', handler);
		return () => window.removeEventListener('application-updated', handler);
	});

	async function refreshDeployment() {
		if (!environment || !organizationId) return;
		deployment = await getDeployment(organizationId, projectId, environment.id, applicationId);
	}

	async function refreshEnvironmentVariables() {
		if (!environment || !organizationId) return;
		const [runtimeVariables, buildVariables] = await Promise.all([
			getEnvironmentVariables(organizationId, projectId, environment.id, applicationId, 'runtime'),
			getEnvironmentVariables(organizationId, projectId, environment.id, applicationId, 'build'),
		]);
		runtimeEnvironmentContent = runtimeVariables
			.map((variable) => `${variable.name}=${variable.value}`)
			.join('\n');
		runtimeVariableNames = runtimeVariables.map((variable) => variable.name);
		buildEnvironmentContent = buildVariables
			.map((variable) => `${variable.name}=${variable.value}`)
			.join('\n');
		await refreshDomainMarkers();
	}

	async function refreshGeneral() {
		if (!environment || !organizationId) return;
		const [latestApplication, latestDeployment, latestSource, latestContainerStatus] =
			await Promise.all([
				getApplication(organizationId, projectId, environment.id, applicationId),
				getDeployment(organizationId, projectId, environment.id, applicationId),
				getApplicationSource(organizationId, projectId, environment.id, applicationId),
				getContainerStatus(organizationId, projectId, environment.id, applicationId),
			]);
		application = latestApplication;
		deployment = latestDeployment;
		domains = latestDeployment.domains;
		source = latestSource;
		containerStatus = latestContainerStatus;
		await refreshDeployHook();
		if (latestSource.deploy_type === 'compose') {
			composeServiceStatuses = await composeStatusApi(
				organizationId,
				projectId,
				environment.id,
				applicationId,
			);
		}
	}

	async function refreshTab(value: string) {
		if (!environment || !organizationId || loading) return;
		refreshingTab = true;
		try {
			if (isDeferredTab(value)) await deferredTabs.refresh(value);
			else if (value === 'general') await refreshGeneral();
			else if (value === 'advanced') await refreshAdvanced();
		} catch (caught) {
			toast.error(caught instanceof Error ? caught.message : `Could not refresh ${value}`);
		} finally {
			refreshingTab = false;
		}
	}
	function mergeDeploymentRun(detail: DeploymentRunDetailDto) {
		if (selectedDeploymentRun?.id !== detail.id) {
			selectedDeploymentRun = { ...detail, logs: detail.logs.slice(-MAX_VISIBLE_DEPLOYMENT_LOGS) };
			return;
		}
		const logs = new Map(selectedDeploymentRun.logs.map((log) => [log.sequence, log]));
		for (const log of detail.logs) logs.set(log.sequence, log);
		selectedDeploymentRun = {
			...detail,
			logs: [...logs.values()]
				.sort((left, right) => left.sequence - right.sequence)
				.slice(-MAX_VISIBLE_DEPLOYMENT_LOGS),
		};
	}

	async function loadDeploymentRun(runId: string) {
		if (!environment) return;
		mergeDeploymentRun(
			await getDeploymentRun(organizationId, projectId, environment.id, applicationId, runId),
		);
	}

	async function selectDeploymentRun(runId: string) {
		if (selectedDeploymentRun?.id === runId) {
			selectedDeploymentRun = null;
			return;
		}
		try {
			await loadDeploymentRun(runId);
		} catch (caught) {
			toast.error(caught instanceof Error ? caught.message : 'Could not load deployment output');
		}
	}

	// "Hard" re-enable: drop whatever state accumulated while paused and pull a
	// clean copy from the server before resuming, instead of just flipping the
	// flag back on and letting live events trickle into stale state.
	async function toggleLiveLogs() {
		if (!liveLogsPaused) {
			liveLogsPaused = true;
			return;
		}
		const runId = selectedDeploymentRun?.id;
		if (runId) {
			selectedDeploymentRun = null;
			await loadDeploymentRun(runId);
		}
		liveLogsPaused = false;
	}

	async function deleteDeploymentRunHandler(runId: string) {
		if (!environment) return;
		await deleteDeploymentRun(organizationId, projectId, environment.id, applicationId, runId);
		deploymentRuns = await listDeploymentRuns(
			organizationId,
			projectId,
			environment.id,
			applicationId,
		);
		if (selectedDeploymentRun?.id === runId) {
			selectedDeploymentRun = null;
			if (deploymentRuns[0]) await loadDeploymentRun(deploymentRuns[0].id);
		}
	}

	async function deleteAllDeploymentRunsHandler() {
		if (!environment) return;
		await deleteDeploymentRuns(organizationId, projectId, environment.id, applicationId);
		deploymentRuns = await listDeploymentRuns(
			organizationId,
			projectId,
			environment.id,
			applicationId,
		);
		selectedDeploymentRun = null;
		if (deploymentRuns[0]) await loadDeploymentRun(deploymentRuns[0].id);
	}

	function stopDeploymentEvents() {
		deploymentEvents?.close();
		deploymentEvents = null;
		deploymentStreamState = 'idle';
		activeDeploymentJobId = null;
		activeDeploymentStatus = null;
		activeDeploymentRunId = null;
	}

	function appendDeploymentLog(log: DeploymentRunLogDto) {
		if (!selectedDeploymentRun) return;
		selectedDeploymentRun = {
			...selectedDeploymentRun,
			logs: [...selectedDeploymentRun.logs, log].slice(-MAX_VISIBLE_DEPLOYMENT_LOGS),
		};
		if (log.server_id) scheduleReplicaRefresh(selectedDeploymentRun.id);
	}

	// Per-server status inside a group run only changes server-side; re-read the
	// run at most every few seconds while its replicas are still writing output.
	let replicaRefreshTimer: ReturnType<typeof setTimeout> | null = null;
	function scheduleReplicaRefresh(runId: string) {
		if (replicaRefreshTimer) return;
		replicaRefreshTimer = setTimeout(() => {
			replicaRefreshTimer = null;
			if (selectedDeploymentRun?.id === runId) void loadDeploymentRun(runId).catch(() => {});
		}, 3000);
	}

	async function retryReplica(serverId: string) {
		if (!environment) return;
		try {
			const job = await retryReplicaDeploy(
				organizationId,
				projectId,
				environment.id,
				applicationId,
				serverId,
			);
			await refreshDeploymentRuns(job.run_id);
			startDeploymentEvents(job);
			toast.success(t('projects.deployment_replicas.retry_queued'));
		} catch (caught) {
			toast.error(
				caught instanceof Error ? caught.message : t('projects.deployment_replicas.retry_failed'),
			);
		}
	}

	// The worker can claim a job before the stream connects, so the `status`
	// events that moved it past `queued` may never arrive; read it back instead.
	async function syncDeploymentJobStatus(jobId: string) {
		const current = (await listDeploymentQueue(organizationId)).find((item) => item.id === jobId);
		if (!current || activeDeploymentJobId !== jobId) return;
		// Only move forward: a live event may have landed while this request ran.
		const rank = (status: string | null) => ACTIVE_JOB_STATUSES.indexOf(status ?? '');
		if (rank(current.status) > rank(activeDeploymentStatus))
			activeDeploymentStatus = current.status;
	}

	function startDeploymentEvents(job: DeploymentJobDto) {
		stopDeploymentEvents();
		activeDeploymentJobId = job.id;
		activeDeploymentStatus = job.status;
		activeDeploymentRunId = job.run_id;
		const events = streamDeploymentQueueEvents();
		deploymentEvents = events;
		events.onopen = () => {
			deploymentStreamRetryCount = 0;
			deploymentStreamState = 'live';
			void loadDeploymentRun(job.run_id);
			void syncDeploymentJobStatus(job.id).catch(() => {});
		};
		events.onerror = () => {
			if (events.readyState === EventSource.CLOSED) return;
			deploymentStreamRetryCount += 1;
			// The browser retries EventSource connections on its own; after a few
			// failures in a row that's clearly not working, so stop relying on it
			// silently and give the user something to act on.
			if (deploymentStreamRetryCount >= 3) {
				events.close();
				deploymentEvents = null;
				deploymentStreamState = 'error';
			} else {
				deploymentStreamState = 'reconnecting';
			}
		};
		const handleQueueEvent = (message: Event) => {
			const event = JSON.parse((message as MessageEvent).data) as DeploymentQueueEvent;
			const action = queueEventAction(event, {
				activeJobId: activeDeploymentJobId,
				livePaused: liveLogsPaused,
				run: selectedDeploymentRun,
				now: Date.now(),
			});
			if (action.type === 'append') appendDeploymentLog(action.log);
			else if (action.type === 'status') activeDeploymentStatus = action.status;
			else if (action.type === 'steps') {
				if (selectedDeploymentRun?.id === job.run_id)
					selectedDeploymentRun = { ...selectedDeploymentRun, steps: action.steps };
			} else if (action.type === 'resync') {
				void loadDeploymentRun(job.run_id);
				void syncDeploymentJobStatus(job.id).catch(() => {});
			} else if (action.type === 'terminal') {
				activeDeploymentStatus = action.status;
				void (async () => {
					await Promise.all([
						loadDeploymentRun(job.run_id),
						refreshDeploymentRuns(job.run_id),
						refreshDeployment(),
						refreshRuntimeStatus(),
						deferredTabs.refresh('volumes'),
					]);
					if (deployment?.status === 'running') toast.success('Application deployed');
					else if (action.status === 'failed') showDeploymentFailure();
					else if (action.status === 'canceled') toast.error('Deployment canceled');
					stopDeploymentEvents();
				})();
			}
		};
		for (const kind of ['log', 'status', 'terminal', 'resync', 'steps']) {
			events.addEventListener(kind, handleQueueEvent);
		}
	}

	async function resumeLiveDeployment() {
		const job = (await listDeploymentQueue(organizationId)).find(
			(item) => item.application_id === applicationId && ACTIVE_JOB_STATUSES.includes(item.status),
		);
		if (!job) return;
		await loadDeploymentRun(job.run_id);
		startDeploymentEvents(job);
	}

	// Called from the "Reconnect" button once the stream gives up on its own.
	async function retryDeploymentStream() {
		deploymentStreamRetryCount = 0;
		try {
			await resumeLiveDeployment();
			if (!deploymentEvents) {
				// No active job anymore — the deploy likely finished while
				// disconnected. Fall back to a plain refresh instead of leaving
				// the error state up with nothing to reconnect to.
				await Promise.all([refreshDeploymentRuns(), refreshDeployment()]);
				deploymentStreamState = 'idle';
			}
		} catch (caught) {
			toast.error(
				caught instanceof Error ? caught.message : 'Could not reconnect to deployment output',
			);
		}
	}

	async function upload(file: File) {
		if (!environment) return;
		busy = true;
		try {
			deployment = await uploadApplicationArtifact(
				organizationId,
				projectId,
				environment.id,
				applicationId,
				file,
			);
			toast.success('Artifact saved and ready to deploy');
		} catch (caught) {
			toast.error(caught instanceof Error ? caught.message : 'Artifact upload failed');
		} finally {
			busy = false;
		}
	}

	async function removeArtifact() {
		if (!environment) return;
		busy = true;
		try {
			deployment = await deleteApplicationArtifact(
				organizationId,
				projectId,
				environment.id,
				applicationId,
			);
			toast.success('Artifact removed');
		} catch (caught) {
			toast.error(caught instanceof Error ? caught.message : 'Artifact removal failed');
		} finally {
			busy = false;
		}
	}

	async function refreshDomains() {
		if (!environment) return;
		domains = await listDomains(organizationId, projectId, environment.id, applicationId);
		deployment = await getDeployment(organizationId, projectId, environment.id, applicationId);
		portConstraints = await getPortConstraints(
			organizationId,
			projectId,
			environment.id,
			applicationId,
		);
		await refreshDomainMarkers();
	}

	/** `{{url}}` env markers only exist in Compose apps; the panel is optional, so failures stay quiet. */
	async function refreshDomainMarkers() {
		if (!environment || source?.deploy_type !== 'compose') {
			domainMarkers = null;
			return;
		}
		domainMarkers = await getDomainMarkers(
			organizationId,
			projectId,
			environment.id,
			applicationId,
		).catch(() => null);
	}

	async function refreshAdvanced() {
		if (!environment) return;
		const [latestSource, latestBasicAuth] = await Promise.all([
			getApplicationSource(organizationId, projectId, environment.id, applicationId),
			getBasicAuth(organizationId, projectId, environment.id, applicationId),
		]);
		source = latestSource;
		basicAuth = latestBasicAuth;
	}

	async function saveBasicAuth(input: Parameters<typeof updateBasicAuth>[4]) {
		if (!environment) {
			throw new Error('No environment');
		}
		basicAuth = await updateBasicAuth(
			organizationId,
			projectId,
			environment.id,
			applicationId,
			input,
		);
		return basicAuth;
	}

	async function addBasicAuthUserHandler(input: { username: string; password: string }) {
		if (!environment) throw new Error('No environment');
		busy = true;
		try {
			const user = await addBasicAuthUser(
				organizationId,
				projectId,
				environment.id,
				applicationId,
				input,
			);
			basicAuth = await getBasicAuth(organizationId, projectId, environment.id, applicationId);
			toast.success(`Added user ${user.username}`);
			return user;
		} catch (caught) {
			toast.error(caught instanceof Error ? caught.message : 'Could not add user');
			throw caught;
		} finally {
			busy = false;
		}
	}

	async function deleteBasicAuthUserHandler(userId: string) {
		if (!environment) throw new Error('No environment');
		busy = true;
		try {
			await deleteBasicAuthUser(organizationId, projectId, environment.id, applicationId, userId);
			basicAuth = await getBasicAuth(organizationId, projectId, environment.id, applicationId);
			toast.success('User removed');
		} catch (caught) {
			toast.error(caught instanceof Error ? caught.message : 'Could not remove user');
			throw caught;
		} finally {
			busy = false;
		}
	}

	// A domain write that changes a published port marks the deployment stale.
	// Catch the false -> true transition so the user is asked to redeploy instead
	// of having to notice the banner.
	async function refreshAndPromptIfStale(port: number | null) {
		const wasStale = deployment?.config_stale ?? false;
		await refreshDomains();
		if (!wasStale && deployment?.config_stale) {
			redeployPort = port;
			redeployOpen = true;
		}
	}

	// The dialogs close only on a resolved promise, so these rethrow after
	// toasting — otherwise a rejected save would close the dialog and lose input.
	async function addDomain(input: DomainInput) {
		if (!environment) return;
		busy = true;
		try {
			const saved = await createDomain(
				organizationId,
				projectId,
				environment.id,
				applicationId,
				input,
			);
			await refreshAndPromptIfStale(input.port);
			toast.success('Domain added');
			invalidateOnboarding();
			return saved;
		} catch (caught) {
			toast.error(caught instanceof Error ? caught.message : 'Could not add domain');
			throw caught;
		} finally {
			busy = false;
		}
	}

	async function validateDns(domainId: string) {
		if (!environment) throw new Error('Environment is unavailable');
		return validateDomainDns(organizationId, projectId, environment.id, applicationId, domainId);
	}

	async function validateHostDnsFor(host: string, serverId: string | null) {
		if (!environment) throw new Error('Environment is unavailable');
		return validateHostDns(
			organizationId,
			projectId,
			environment.id,
			applicationId,
			host,
			serverId,
		);
	}

	async function editDomain(domainId: string, input: DomainInput) {
		if (!environment) return;
		busy = true;
		try {
			const saved = await updateDomain(
				organizationId,
				projectId,
				environment.id,
				applicationId,
				domainId,
				input,
			);
			await refreshAndPromptIfStale(input.port);
			toast.success('Domain saved');
			return saved;
		} catch (caught) {
			toast.error(caught instanceof Error ? caught.message : 'Could not save domain');
			throw caught;
		} finally {
			busy = false;
		}
	}

	async function removeDomain(domainId: string) {
		if (!environment) return;
		busy = true;
		try {
			await deleteDomain(organizationId, projectId, environment.id, applicationId, domainId);
			await refreshAndPromptIfStale(null);
			toast.success('Domain deleted');
		} catch (caught) {
			toast.error(caught instanceof Error ? caught.message : 'Could not delete domain');
			throw caught;
		} finally {
			busy = false;
		}
	}
	async function suggestRandomSslipHost() {
		if (!environment) throw new Error('Application is not ready');
		return randomSslipHost(organizationId, projectId, environment.id, applicationId);
	}
	async function saveComposeDomainRoute(domainId: string, service: string, containerPort: number) {
		if (!environment) return;
		const saved = await saveComposeProxyRoute(
			organizationId,
			projectId,
			environment.id,
			applicationId,
			{
				domain_id: domainId,
				service,
				container_port: containerPort,
			},
		);
		composeProxyRoutes = [
			...composeProxyRoutes.filter((route) => route.domain_id !== domainId),
			saved,
		];
		await refreshDomainMarkers();
		toast.success('Caddy target saved');
	}

	async function makeDomain() {
		if (!environment) return;
		busy = true;
		try {
			await generateDomain(organizationId, projectId, environment.id, applicationId);
			await refreshDomains();
			toast.success('Domain generated');
		} catch (caught) {
			toast.error(caught instanceof Error ? caught.message : 'Could not generate domain');
		} finally {
			busy = false;
		}
	}

	async function saveEnvironment(scope: 'runtime' | 'build', content: string) {
		if (!environment) {
			const error = new Error('Environment is unavailable');
			toast.error(error.message);
			throw error;
		}
		busy = true;
		try {
			const variables = await saveEnvironmentVariables(
				organizationId,
				projectId,
				environment.id,
				applicationId,
				content,
				scope,
			);
			const saved = variables.map((variable) => `${variable.name}=${variable.value}`).join('\n');
			if (scope === 'runtime') {
				runtimeEnvironmentContent = saved;
				runtimeVariableNames = variables.map((variable) => variable.name);
				await refreshDomainMarkers();
			} else buildEnvironmentContent = saved;
			toast.success('Environment saved');
		} catch (caught) {
			toast.error(caught instanceof Error ? caught.message : 'Could not save environment');
			throw caught;
		} finally {
			busy = false;
		}
	}

	/**
	 * Appends variables the Compose file references but the runtime scope lacks.
	 *
	 * The API replaces the whole scope on save and deletes anything missing from
	 * the payload, so the current (masked) document has to go along for the ride.
	 * Masked values are left untouched server-side, so re-sending them is safe.
	 */
	async function handleCreateRuntimeVariables(created: { name: string; value: string }[]) {
		if (!created.length) return;
		if (!runtimeEnvironmentContent) await refreshEnvironmentVariables();
		const additions = created.map((variable) => `${variable.name}=${variable.value}`);
		const merged = [runtimeEnvironmentContent, ...additions].filter(Boolean).join('\n');
		await saveEnvironment('runtime', merged);
	}

	async function revealEnvironment(scope: 'runtime' | 'build') {
		if (!environment) {
			const error = new Error('Environment is unavailable');
			toast.error(error.message);
			throw error;
		}
		try {
			const variables = await getEnvironmentVariables(
				organizationId,
				projectId,
				environment.id,
				applicationId,
				scope,
				true,
			);
			return variables.map((variable) => `${variable.name}=${variable.value}`).join('\n');
		} catch (caught) {
			toast.error(caught instanceof Error ? caught.message : 'Could not reveal environment');
			throw caught;
		}
	}

	async function deploy(cleanBuild?: boolean) {
		if (!environment) return;
		busy = true;
		try {
			const result = await deployApplication(
				organizationId,
				projectId,
				environment.id,
				applicationId,
				cleanBuild,
			);
			if ('run_id' in result) {
				await refreshDeployment();
				domains = deployment?.domains ?? domains;
				selectTab('deployment');
				await refreshDeploymentRuns(result.run_id);
				startDeploymentEvents(result);
				toast.success('Deployment queued — streaming build output');
			} else {
				deployment = result;
				domains = deployment.domains;
				await refreshRuntimeStatus();
				await deferredTabs.refresh('volumes');
				toast.success('Application deployed');
			}
		} catch (caught) {
			showDeploymentFailure();
			try {
				await refreshDeployment();
			} catch {
				/* the toast above already reported the failure */
			}
		} finally {
			busy = false;
		}
	}

	async function rollbackRun(runId: string) {
		if (!environment) return;
		busy = true;
		try {
			const job = await rollbackDeploymentRun(
				organizationId,
				projectId,
				environment.id,
				applicationId,
				runId,
			);
			await refreshDeployment();
			domains = deployment?.domains ?? domains;
			await refreshDeploymentRuns(job.run_id);
			startDeploymentEvents(job);
			toast.success('Rollback queued — streaming deploy output');
		} catch (caught) {
			showDeploymentFailure();
			try {
				await refreshDeployment();
			} catch {
				/* the toast above already reported the failure */
			}
		} finally {
			busy = false;
		}
	}

	async function refreshDeploymentRuns(selectedRunId?: string) {
		if (!environment) return;
		// A refresh (Refresh button, tab switch, live-stream events) must never
		// steal the run the user is currently looking at. Only the very first
		// load (nothing fetched yet, nothing selected) gets the convenience of
		// auto-opening the newest run; every later refresh keeps whatever is
		// already selected - or stays collapsed if the user collapsed it.
		const firstLoad = deploymentRuns.length === 0;
		deploymentRuns = await listDeploymentRuns(
			organizationId,
			projectId,
			environment.id,
			applicationId,
		);
		const runId =
			selectedRunId ?? selectedDeploymentRun?.id ?? (firstLoad ? deploymentRuns[0]?.id : undefined);
		if (runId) await loadDeploymentRun(runId);
	}

	async function saveSource(input: SourceConfigDto) {
		if (!environment) return;
		busy = true;
		try {
			source = await saveApplicationSource(
				organizationId,
				projectId,
				environment.id,
				applicationId,
				input,
			);
			await refreshDeployment();
			toast.success('Source saved');
		} catch (caught) {
			toast.error(caught instanceof Error ? caught.message : 'Could not save source');
			throw caught;
		} finally {
			busy = false;
		}
	}

	async function saveHealthCheck(input: UpdateHealthCheckInput) {
		if (!environment) return;
		busy = true;
		try {
			healthCheck = await updateHealthCheck(
				organizationId,
				projectId,
				environment.id,
				applicationId,
				input,
			);
			toast.success(t('projects.health_check.saved'));
		} catch (caught) {
			toast.error(
				caught instanceof Error ? caught.message : t('projects.health_check.save_failed'),
			);
			throw caught;
		} finally {
			busy = false;
		}
	}

	async function handleSetCleanBuild(value: boolean) {
		if (!environment) return;
		busy = true;
		try {
			source = await setCleanBuild(organizationId, projectId, environment.id, applicationId, value);
		} catch (caught) {
			toast.error(caught instanceof Error ? caught.message : 'Could not update clean build');
			throw caught;
		} finally {
			busy = false;
		}
	}

	async function refreshDeployHook() {
		if (!environment) return;
		try {
			deployHook = await getDeployHook(organizationId, projectId, environment.id, applicationId);
		} catch {
			deployHook = null;
		}
	}

	async function rotateHook() {
		if (!environment) return;
		busy = true;
		try {
			deployHook = await rotateDeployHook(organizationId, projectId, environment.id, applicationId);
			toast.success('Deploy URL rotated — the old one no longer works');
		} catch (caught) {
			toast.error(caught instanceof Error ? caught.message : 'Could not rotate the deploy URL');
		} finally {
			busy = false;
		}
	}

	async function loadRepositories(providerId: string): Promise<GitRepositoryDto[]> {
		try {
			return await listGitRepositories(organizationId, providerId);
		} catch (caught) {
			toast.error(caught instanceof Error ? caught.message : 'Could not load repositories');
			return [];
		}
	}

	async function loadBranches(providerId: string, repoUrl: string): Promise<string[]> {
		try {
			return await listGitBranches(organizationId, providerId, repoUrl);
		} catch (caught) {
			toast.error(caught instanceof Error ? caught.message : 'Could not load branches');
			return [];
		}
	}

	async function loadPublicBranches(repoUrl: string): Promise<string[]> {
		try {
			return await listPublicGitBranches(organizationId, repoUrl);
		} catch (caught) {
			toast.error(caught instanceof Error ? caught.message : 'Could not load branches');
			return [];
		}
	}

	async function stop() {
		if (!environment) return;
		busy = true;
		try {
			deployment = await stopApplication(organizationId, projectId, environment.id, applicationId);
			await refreshRuntimeStatus();
			toast.success('Application stopped');
		} catch (caught) {
			toast.error(caught instanceof Error ? caught.message : 'Could not stop application');
		} finally {
			busy = false;
		}
	}

	async function start() {
		if (!environment) return;
		busy = true;
		try {
			deployment = await startApplication(organizationId, projectId, environment.id, applicationId);
			await refreshRuntimeStatus();
			toast.success('Application started');
		} catch (caught) {
			toast.error(caught instanceof Error ? caught.message : 'Could not start application');
		} finally {
			busy = false;
		}
	}

	async function restart() {
		if (!environment) return;
		busy = true;
		try {
			await stopApplication(organizationId, projectId, environment.id, applicationId);
			deployment = await startApplication(organizationId, projectId, environment.id, applicationId);
			await refreshRuntimeStatus();
			toast.success('Application restarted');
		} catch (caught) {
			toast.error(caught instanceof Error ? caught.message : 'Could not restart application');
		} finally {
			busy = false;
		}
	}

	async function removeRuntime() {
		if (!environment) return;
		busy = true;
		try {
			deployment = await removeApplicationRuntime(
				organizationId,
				projectId,
				environment.id,
				applicationId,
			);
			await refreshRuntimeStatus();
			toast.success('Application runtime deleted');
		} catch (caught) {
			toast.error(
				caught instanceof Error ? caught.message : 'Could not delete application runtime',
			);
		} finally {
			busy = false;
		}
	}

	async function handleComposeSave(content: string, portMappings: PortMapping[]) {
		if (!environment) throw new Error('Application is not ready');
		busy = true;
		try {
			const saved = await composeContentPut(
				organizationId,
				projectId,
				environment.id,
				applicationId,
				{
					content,
					port_mappings: portMappings,
				},
			);
			composeContent = saved.content;
			composePortMappings = saved.port_mappings;
			// Keep the domain tab's service dropdown in sync with services the
			// user just added/removed in the file, without a full page reload.
			composeServices = saved.catalog?.services ?? [];
			toast.success('Compose file saved');
			return saved;
		} catch (caught) {
			toast.error(caught instanceof Error ? caught.message : 'Failed to save compose file');
			throw caught;
		} finally {
			busy = false;
		}
	}

	async function handleComposeValidate(content: string): Promise<ValidationResponse> {
		if (!environment) return { valid: false, errors: [{ message: 'Not ready' }] };
		try {
			return await composeValidate(
				organizationId,
				projectId,
				environment.id,
				applicationId,
				content,
			);
		} catch (caught) {
			return {
				valid: false,
				errors: [{ message: caught instanceof Error ? caught.message : 'Validation failed' }],
			};
		}
	}

	async function handleComposeDeploy() {
		if (!environment) return;
		// Redeploys hit live containers; force the user to confirm by typing
		// the application name. First-time deploys have nothing to recreate and
		// run straight through the queue.
		if (hasRunningContainer && application) {
			redeployWithPullOpen = true;
			return;
		}
		await runComposeDeploy();
	}

	async function runComposeDeploy(confirmation?: string) {
		if (!environment) return;
		selectTab('deployment');
		busy = true;
		try {
			const job = await composeDeploy(
				organizationId,
				projectId,
				environment.id,
				applicationId,
				confirmation,
			);
			await refreshDeployment();
			await refreshDeploymentRuns(job.run_id);
			startDeploymentEvents(job);
			toast.success('Deployment queued — streaming Compose output');
		} catch (caught) {
			showDeploymentFailure();
			try {
				await refreshDeployment();
				await refreshDeploymentRuns();
			} catch {
				// Keep the deploy error visible if the history refresh also fails.
			}
		} finally {
			busy = false;
		}
	}

	async function handleComposeStop() {
		if (!environment) return;
		busy = true;
		try {
			deployment = await composeStopApi(organizationId, projectId, environment.id, applicationId);
			toast.success('Compose services stopped');
			await refreshRuntimeStatus();
		} catch (caught) {
			toast.error(caught instanceof Error ? caught.message : 'Stop failed');
		} finally {
			busy = false;
		}
	}

	async function handleComposeStart() {
		if (!environment) return;
		busy = true;
		try {
			deployment = await composeStartApi(organizationId, projectId, environment.id, applicationId);
			toast.success('Compose services started');
			await refreshRuntimeStatus();
		} catch (caught) {
			toast.error(caught instanceof Error ? caught.message : 'Start failed');
		} finally {
			busy = false;
		}
	}

	function openContainerTerminal(containerName: string) {
		activeTab = 'terminal';
		const url = new URL(page.url.href);
		url.searchParams.set('tab', 'terminal');
		url.searchParams.set('container', containerName);
		goto(url, { shallow: true, replace: true, state: page.state });
	}

	async function refreshVolumeState() {
		if (!environment) return;
		busy = true;
		try {
			deployment = await getDeployment(organizationId, projectId, environment.id, applicationId);
			managedVolumes = await listManagedVolumes(
				organizationId,
				projectId,
				environment.id,
				applicationId,
			);
			// Always ask the server, instead of gating on deployment.status here:
			// for remote/compose apps that column can lag the live container
			// state, and the API already returns [] on its own when nothing is
			// actually running (mod.rs `volumes()` checks the real state itself).
			volumes = await getApplicationVolumes(
				organizationId,
				projectId,
				environment.id,
				applicationId,
			);
		} catch (caught) {
			toast.error(caught instanceof Error ? caught.message : 'Could not refresh volumes');
		} finally {
			busy = false;
		}
	}

	async function handleCreateManagedVolume(input: CreateManagedVolumeInput) {
		if (!environment) return;
		await createManagedVolume(organizationId, projectId, environment.id, applicationId, input);
		await refreshVolumeState();
		toast.success('Volume created');
	}
	async function handleAttachManagedVolume(volumeId: string) {
		if (!environment) return;
		await attachManagedVolume(organizationId, projectId, environment.id, applicationId, volumeId);
		await refreshVolumeState();
		toast.success('Volume will attach on the next deployment');
	}
	async function handleDetachManagedVolume(volumeId: string) {
		if (!environment) return;
		await detachManagedVolume(organizationId, projectId, environment.id, applicationId, volumeId);
		await refreshVolumeState();
		toast.success('Volume detached; its files are retained');
	}
	async function handleDeleteManagedVolume(volumeId: string) {
		if (!environment) return;
		await deleteManagedVolume(organizationId, projectId, environment.id, applicationId, volumeId);
		await refreshVolumeState();
		toast.success('Volume permanently deleted');
	}

	async function handleEditApplication(input: {
		name: string;
		description: string;
		icon_url?: string | null;
	}) {
		if (!environment) return;
		await updateApplication(organizationId, projectId, environment.id, applicationId, input);
		application = {
			...application!,
			name: input.name,
			description: input.description,
			icon_url: input.icon_url === undefined ? application!.icon_url : input.icon_url,
		};
		toast.success('Application updated');
	}
	async function handleUploadApplicationIcon(file: File) {
		if (!environment) throw new Error('Environment not loaded');
		const updated = await uploadApplicationIcon(
			organizationId,
			projectId,
			environment.id,
			applicationId,
			file,
		);
		return updated.icon_url ?? '';
	}
	async function handleDeleteApplication() {
		if (!environment) return;
		await deleteApplication(organizationId, projectId, environment.id, applicationId);
		goto(
			`/projects/${encodeURIComponent(projectId)}/environments/${encodeURIComponent(environment.id)}`,
		);
	}
	function workspaceFor(source: string) {
		return applicationVolumeWorkspace(
			organizationId,
			projectId,
			environment!.id,
			applicationId,
			source,
		);
	}
	function managedWorkspaceFor(volumeId: string) {
		return managedVolumeWorkspace(
			organizationId,
			projectId,
			environment!.id,
			applicationId,
			volumeId,
		);
	}
</script>

{#snippet appCrumbIcon()}
	<ApplicationIcon
		templateKind={application?.template_kind ?? null}
		catalogIconUrl={application?.catalog_icon_url ?? null}
		iconUrl={application?.icon_url ?? null}
		applicationId={application?.id ?? ''}
		bind:failedApplicationIds={failedIconApplicationIds}
		class="size-3.5" />
{/snippet}

{#snippet projectItemIcon(id: string)}
	<ProjectIcon iconUrl={projects.find((item) => item.id === id)?.icon_url} />
{/snippet}

{#snippet appItemIcon(id: string)}
	{@const item = environment?.applications.find((candidate) => candidate.id === id)}
	{#if item}
		<ApplicationIcon
			templateKind={item.template_kind}
			catalogIconUrl={item.catalog_icon_url}
			iconUrl={item.id === application?.id ? application.icon_url : item.icon_url}
			applicationId={item.id}
			bind:failedApplicationIds={failedIconApplicationIds}
			class="size-3.5 shrink-0" />
	{/if}
{/snippet}

{#snippet projectCrumbIcon()}
	{#if isDisplayableImageUrl(projectDetail?.project.icon_url) && !projectIconFailed}
		<img
			src={projectDetail?.project.icon_url}
			alt=""
			class="size-4 rounded-[4px] object-contain"
			onerror={() => (projectIconFailed = true)} />
	{:else}
		<FolderKanbanIcon class="text-muted-foreground size-3.5" />
	{/if}
{/snippet}

{#snippet heroAvatar()}
	{#if application}
		<span class="grid size-full place-items-center bg-muted">
			<ApplicationIcon
				templateKind={application.template_kind}
				catalogIconUrl={application.catalog_icon_url}
				iconUrl={application.icon_url}
				applicationId={application.id}
				bind:failedApplicationIds={failedIconApplicationIds}
				class="size-6" />
		</span>
	{/if}
{/snippet}

{#snippet heroStatus()}
	{#if deployment}
		<span
			class="rounded-full px-2 py-0.5 text-xs font-medium {hasRunningContainer
				? 'bg-success/12 text-success'
				: 'bg-muted text-muted-foreground'}"
			>{hasRunningContainer
				? t('projects.environment.running')
				: t('projects.environment.stopped')}</span>
	{/if}
{/snippet}

{#snippet heroMeta()}
	{#if environment}<span>{environment.name}</span>{/if}
	{#if application?.description}
		<span aria-hidden="true">·</span>
		<span class="line-clamp-1 break-words">{application.description}</span>
	{/if}
	{#if application?.server}
		<ServerMark server={application.server} />
	{:else if application?.server_group}
		<ServerGroupMark group={application.server_group} />
	{/if}
{/snippet}

{#snippet heroActions()}
	<Tooltip.Root delayDuration={200}>
		<Tooltip.Trigger>
			{#snippet child({ props })}
				<Button
					{...props}
					variant="ghost"
					size="icon-sm"
					onclick={() => (editApplicationOpen = true)}
					aria-label={t('projects.application.edit')}><PencilIcon /></Button>
			{/snippet}
		</Tooltip.Trigger>
		<Tooltip.Content class="max-w-xs">{t('projects.application.edit_hint')}</Tooltip.Content>
	</Tooltip.Root>
	<Tooltip.Root delayDuration={200}>
		<Tooltip.Trigger>
			{#snippet child({ props })}
				<Button
					{...props}
					variant="ghost"
					size="icon-sm"
					class="text-destructive hover:text-destructive"
					onclick={() => (deleteDialogOpen = true)}
					aria-label={t('projects.application.delete')}><Trash2Icon /></Button>
			{/snippet}
		</Tooltip.Trigger>
		<Tooltip.Content class="max-w-xs">{t('projects.application.delete_hint')}</Tooltip.Content>
	</Tooltip.Root>
	{#if deployment}
		{#if hasRunningContainer}
			<Button variant="outline" size="sm" disabled={busy} onclick={() => void stop()}>
				<SquareIcon />{t('projects.environment.stop')}
			</Button>
		{:else}
			<Button variant="outline" size="sm" disabled={busy} onclick={() => void start()}>
				<PlayIcon />{t('projects.environment.start')}
			</Button>
		{/if}
		<Button size="sm" disabled={busy} onclick={() => void deploy()}>
			<RefreshCwIcon class={busy ? 'animate-spin' : ''} />{t('projects.environment.redeploy')}
		</Button>
	{/if}
{/snippet}

<svelte:window onkeydown={handleTabShortcut} />

<div class="flex w-full min-w-0 flex-col gap-6 px-4 py-6 sm:px-6">
	{#if loading}
		<p class="text-sm text-muted-foreground">{t('projects.application.loading')}</p>
	{:else if error}
		<div class="rounded-lg border border-destructive/30 bg-destructive/5 p-4">
			<p class="font-medium">{t('projects.application.unable_to_load')}</p>
			<p class="mt-1 text-sm text-muted-foreground">{error}</p>
		</div>
	{:else if application && environment && projectDetail}
		{#if deployment}
			<div class="mt-2 min-w-0 max-w-full">
				{#if refreshingTab && activeTab !== 'general' && activeTab !== 'deployment'}
					<SkeletonRows rows={4} height="h-20" label="Loading tab" />
				{:else if activeTab === 'general' && application.template_kind === 'postgres'}
					<PostgresResourceDetail
						{organizationId}
						{projectId}
						environmentId={environment.id}
						{applicationId}
						onDeploy={deploy}
						onStart={start}
						onStop={stop}
						onRestart={restart}
						runtimeBusy={busy}
						deploymentStatus={deployment.status}
						{composeContent}
						{composePortMappings}
						{runtimeVariableNames}
						onComposeSave={handleComposeSave}
						onComposeDeploy={handleComposeDeploy}
						onComposeValidate={handleComposeValidate}
						onComposeContentChange={(content) => (composeContent = content)}
						onCreateRuntimeVariables={handleCreateRuntimeVariables} />
				{:else if activeTab === 'general' && application.template_kind === 'mariadb'}
					<MariaDbResourceDetail
						{organizationId}
						{projectId}
						environmentId={environment.id}
						{applicationId}
						onDeploy={deploy}
						onStart={start}
						onStop={stop}
						onRestart={restart}
						runtimeBusy={busy}
						deploymentStatus={deployment.status}
						{composeContent}
						{composePortMappings}
						{runtimeVariableNames}
						onComposeSave={handleComposeSave}
						onComposeDeploy={handleComposeDeploy}
						onComposeValidate={handleComposeValidate}
						onComposeContentChange={(content) => (composeContent = content)}
						onCreateRuntimeVariables={handleCreateRuntimeVariables} />
				{:else if activeTab === 'general' && application.template_kind === 'dragonflydb'}
					<DragonflyDbResourceDetail
						{organizationId}
						{projectId}
						environmentId={environment.id}
						{applicationId}
						onDeploy={deploy}
						onStart={start}
						onStop={stop}
						onRestart={restart}
						runtimeBusy={busy}
						deploymentStatus={deployment.status}
						{composeContent}
						{composePortMappings}
						{runtimeVariableNames}
						onComposeSave={handleComposeSave}
						onComposeDeploy={handleComposeDeploy}
						onComposeValidate={handleComposeValidate}
						onComposeContentChange={(content) => (composeContent = content)}
						onCreateRuntimeVariables={handleCreateRuntimeVariables} />
				{:else if activeTab === 'general' && application.template_kind === 'keydb'}
					<KeyDbResourceDetail
						{organizationId}
						{projectId}
						environmentId={environment.id}
						{applicationId}
						onDeploy={deploy}
						onStart={start}
						onStop={stop}
						onRestart={restart}
						runtimeBusy={busy}
						deploymentStatus={deployment.status}
						{composeContent}
						{composePortMappings}
						{runtimeVariableNames}
						onComposeSave={handleComposeSave}
						onComposeDeploy={handleComposeDeploy}
						onComposeValidate={handleComposeValidate}
						onComposeContentChange={(content) => (composeContent = content)}
						onCreateRuntimeVariables={handleCreateRuntimeVariables} />
				{:else if activeTab === 'general' && application.template_kind === 'sqlserver'}
					<SqlServerResourceDetail
						{organizationId}
						{projectId}
						environmentId={environment.id}
						{applicationId}
						onDeploy={deploy}
						onStart={start}
						onStop={stop}
						onRestart={restart}
						runtimeBusy={busy}
						deploymentStatus={deployment.status}
						{composeContent}
						{composePortMappings}
						{runtimeVariableNames}
						onComposeSave={handleComposeSave}
						onComposeDeploy={handleComposeDeploy}
						onComposeValidate={handleComposeValidate}
						onComposeContentChange={(content) => (composeContent = content)}
						onCreateRuntimeVariables={handleCreateRuntimeVariables} />
				{:else if activeTab === 'general' && application.template_kind === 'mongodb'}
					<MongoDbResourceDetail
						{organizationId}
						{projectId}
						environmentId={environment.id}
						{applicationId}
						onDeploy={deploy}
						onStart={start}
						onStop={stop}
						onRestart={restart}
						runtimeBusy={busy}
						deploymentStatus={deployment.status}
						{composeContent}
						{composePortMappings}
						{runtimeVariableNames}
						onComposeSave={handleComposeSave}
						onComposeDeploy={handleComposeDeploy}
						onComposeValidate={handleComposeValidate}
						onComposeContentChange={(content) => (composeContent = content)}
						onCreateRuntimeVariables={handleCreateRuntimeVariables} />
				{:else if activeTab === 'backups' && isDatabaseBackupSupported}
					<DatabaseBackupTab
						{organizationId}
						{projectId}
						environmentId={environment.id}
						{applicationId}
						engine={application.template_kind ?? ''} />
				{:else if activeTab === 'general' && source}
					<ApplicationGeneralTab
						{deployment}
						{source}
						providers={gitProviders}
						{busy}
						onUpload={upload}
						onRemoveArtifact={removeArtifact}
						onDeploy={deploy}
						onStop={stop}
						onStart={start}
						onRemoveRuntime={removeRuntime}
						{containerStatus}
						{containerMetrics}
						onSaveSource={saveSource}
						onSetCleanBuild={handleSetCleanBuild}
						onLoadBranches={loadBranches}
						onLoadPublicBranches={loadPublicBranches}
						onLoadRepositories={loadRepositories}
						{deployHook}
						onRotateDeployHook={rotateHook}
						{composeContent}
						{composePortMappings}
						composeStatus={composeServiceStatuses}
						{runtimeVariableNames}
						onComposeSave={handleComposeSave}
						onComposeDeploy={handleComposeDeploy}
						onComposeValidate={handleComposeValidate}
						onComposeStop={handleComposeStop}
						onComposeStart={handleComposeStart}
						onComposeContentChange={(content) => (composeContent = content)}
						onCreateRuntimeVariables={handleCreateRuntimeVariables}
						{templateDocsUrl} />
				{:else if activeTab === 'advanced' && application.template_kind === 'postgres'}
					<PostgresResourceDetail
						{organizationId}
						{projectId}
						environmentId={environment.id}
						{applicationId}
						onDeploy={deploy}
						onStart={start}
						onStop={stop}
						onRestart={restart}
						runtimeBusy={busy}
						deploymentStatus={deployment.status}
						{composeContent}
						{composePortMappings}
						{runtimeVariableNames}
						onComposeSave={handleComposeSave}
						onComposeDeploy={handleComposeDeploy}
						onComposeValidate={handleComposeValidate}
						onComposeContentChange={(content) => (composeContent = content)}
						onCreateRuntimeVariables={handleCreateRuntimeVariables}
						mode="configuration" />
				{:else if activeTab === 'advanced' && application.template_kind === 'mariadb'}
					<MariaDbResourceDetail
						{organizationId}
						{projectId}
						environmentId={environment.id}
						{applicationId}
						onDeploy={deploy}
						onStart={start}
						onStop={stop}
						onRestart={restart}
						runtimeBusy={busy}
						deploymentStatus={deployment.status}
						{composeContent}
						{composePortMappings}
						{runtimeVariableNames}
						onComposeSave={handleComposeSave}
						onComposeDeploy={handleComposeDeploy}
						onComposeValidate={handleComposeValidate}
						onComposeContentChange={(content) => (composeContent = content)}
						onCreateRuntimeVariables={handleCreateRuntimeVariables}
						mode="configuration" />
				{:else if activeTab === 'advanced' && application.template_kind === 'dragonflydb'}
					<DragonflyDbResourceDetail
						{organizationId}
						{projectId}
						environmentId={environment.id}
						{applicationId}
						onDeploy={deploy}
						onStart={start}
						onStop={stop}
						onRestart={restart}
						runtimeBusy={busy}
						deploymentStatus={deployment.status}
						{composeContent}
						{composePortMappings}
						{runtimeVariableNames}
						onComposeSave={handleComposeSave}
						onComposeDeploy={handleComposeDeploy}
						onComposeValidate={handleComposeValidate}
						onComposeContentChange={(content) => (composeContent = content)}
						onCreateRuntimeVariables={handleCreateRuntimeVariables}
						mode="configuration" />
				{:else if activeTab === 'advanced' && application.template_kind === 'keydb'}
					<KeyDbResourceDetail
						{organizationId}
						{projectId}
						environmentId={environment.id}
						{applicationId}
						onDeploy={deploy}
						onStart={start}
						onStop={stop}
						onRestart={restart}
						runtimeBusy={busy}
						deploymentStatus={deployment.status}
						{composeContent}
						{composePortMappings}
						{runtimeVariableNames}
						onComposeSave={handleComposeSave}
						onComposeDeploy={handleComposeDeploy}
						onComposeValidate={handleComposeValidate}
						onComposeContentChange={(content) => (composeContent = content)}
						onCreateRuntimeVariables={handleCreateRuntimeVariables}
						mode="configuration" />
				{:else if activeTab === 'advanced' && application.template_kind === 'sqlserver'}
					<SqlServerResourceDetail
						{organizationId}
						{projectId}
						environmentId={environment.id}
						{applicationId}
						onDeploy={deploy}
						onStart={start}
						onStop={stop}
						onRestart={restart}
						runtimeBusy={busy}
						deploymentStatus={deployment.status}
						{composeContent}
						{composePortMappings}
						{runtimeVariableNames}
						onComposeSave={handleComposeSave}
						onComposeDeploy={handleComposeDeploy}
						onComposeValidate={handleComposeValidate}
						onComposeContentChange={(content) => (composeContent = content)}
						onCreateRuntimeVariables={handleCreateRuntimeVariables}
						mode="configuration" />
				{:else if activeTab === 'advanced' && application.template_kind === 'mongodb'}
					<MongoDbResourceDetail
						{organizationId}
						{projectId}
						environmentId={environment.id}
						{applicationId}
						onDeploy={deploy}
						onStart={start}
						onStop={stop}
						onRestart={restart}
						runtimeBusy={busy}
						deploymentStatus={deployment.status}
						{composeContent}
						{composePortMappings}
						{runtimeVariableNames}
						onComposeSave={handleComposeSave}
						onComposeDeploy={handleComposeDeploy}
						onComposeValidate={handleComposeValidate}
						onComposeContentChange={(content) => (composeContent = content)}
						onCreateRuntimeVariables={handleCreateRuntimeVariables}
						mode="configuration" />
				{:else if activeTab === 'advanced' && source && basicAuth}<div class="grid gap-6">
						<ApplicationAdvancedTab
							server={application.server}
							onLoadResourceBaseline={() =>
								getResourceBaseline(organizationId, projectId, environment!.id, applicationId)}
							{source}
							{domains}
							{basicAuth}
							{busy}
							onSaveSource={saveSource}
							onSaveBasicAuth={saveBasicAuth}
							onAddBasicAuthUser={addBasicAuthUserHandler}
							onDeleteBasicAuthUser={deleteBasicAuthUserHandler}
							{healthCheck}
							onSaveHealthCheck={saveHealthCheck} />
						{#if environment && organizationId}
							<ApplicationBootPolicyCard
								{organizationId}
								{projectId}
								environmentId={environment.id}
								{applicationId} />
						{/if}
					</div>
				{:else if activeTab === 'deployment'}<ApplicationDeploymentTab
						runs={deploymentRuns}
						selected={selectedDeploymentRun}
						liveStatus={selectedDeploymentRun?.id === activeDeploymentRunId
							? activeDeploymentStatus
							: null}
						streamState={deploymentStreamState}
						applicationName={application.name}
						onSelect={selectDeploymentRun}
						onDeleteRun={deleteDeploymentRunHandler}
						onDeleteAll={deleteAllDeploymentRunsHandler}
						onRefresh={() => refreshTab('deployment')}
						onRedeploy={() => deploy()}
						onRollback={rollbackRun}
						refreshing={refreshingTab}
						livePaused={liveLogsPaused}
						onToggleLive={toggleLiveLogs}
						onRetryStream={retryDeploymentStream}
						onRetryReplica={retryReplica}
						onAskAssistant={(text) =>
							askAboutLogs(
								projectDetail?.project.name ?? '',
								application?.name ?? '',
								'deployment',
								text,
							)} />
				{:else if activeTab === 'domains' && !isInfrastructureTemplate}
					<ApplicationDomainsTab
						{domains}
						server={domainServer ?? application.server}
						{pinServers}
						{busy}
						refreshing={refreshingTab}
						configStale={deployment.config_stale}
						constraints={portConstraints}
						onRefresh={() => refreshTab('domains')}
						onCreate={addDomain}
						onUpdate={editDomain}
						onDelete={removeDomain}
						onGenerate={makeDomain}
						onValidateDns={validateDns}
						onValidateHostDns={validateHostDnsFor}
						onDeploy={deploy}
						onRandomSslipHost={suggestRandomSslipHost}
						composeServices={source?.deploy_type === 'compose' ? composeServices : []}
						composeRoutes={composeProxyRoutes}
						{domainMarkers}
						onSaveComposeRoute={saveComposeDomainRoute} />{:else if activeTab === 'environment'}<ApplicationEnvironmentTab
						runtimeContent={runtimeEnvironmentContent}
						buildContent={buildEnvironmentContent}
						{busy}
						onSave={saveEnvironment}
						onReveal={revealEnvironment}
						{domainMarkers} />{:else if activeTab === 'containers'}<div class="flex flex-col gap-6">
						<ApplicationContainersTab
							{containerStatus}
							composeServices={composeServiceStatuses}
							{containerMetrics}
							deployType={source?.deploy_type}
							{busy}
							isCompose={source?.deploy_type === 'compose'}
							isGroup={!!application?.server_group}
							{organizationId}
							{projectId}
							environmentId={environment.id}
							{applicationId}
							onRefresh={() => refreshRuntimeStatus(source?.deploy_type)}
							onComposeStart={handleComposeStart}
							onComposeStop={handleComposeStop}
							onOpenTerminal={openContainerTerminal} />
						{#if hasNetworkData || refreshingTab || source?.deploy_type === 'compose'}
							<ApplicationNetworksTab
								overview={networkOverview}
								loading={refreshingTab}
								error={networkError}
								isCompose={source?.deploy_type === 'compose'}
								onRefresh={() => deferredTabs.refresh('networks')}
								onMakePrivateNetworkExplicit={makePrivateNetworkEditable}
								{resourceLinks}
								{resourceConsumers}
								{resourceLinksLoading}
								isResource={isInfrastructureTemplate}
								onToggleResourceLink={toggleResourceLink} />
							<ApplicationMapTab
								overview={networkOverview}
								statuses={mapStatuses}
								organizationId={organizationId ?? ''}
								serverId={application?.server?.id ?? null}
								loading={refreshingTab}
								error={networkError}
								onRefresh={() => deferredTabs.refresh('map')}
								onProbe={probeNetworks}
								onStats={statsNetworks} />
						{/if}
					</div>
				{:else if activeTab === 'volumes'}
					<ApplicationVolumesTab
						{volumes}
						{managedVolumes}
						isCompose={source?.deploy_type === 'compose' ||
							(source?.deploy_type === 'git' && source?.build_method === 'compose')}
						composeServices={composeServices.map((service) => service.name)}
						{busy}
						onRefresh={refreshVolumeState}
						onCreateManagedVolume={handleCreateManagedVolume}
						onAttachManagedVolume={handleAttachManagedVolume}
						onDetachManagedVolume={handleDetachManagedVolume}
						onDeleteManagedVolume={handleDeleteManagedVolume}
						{workspaceFor}
						{managedWorkspaceFor} />
				{:else if activeTab === 'terminal'}
					<ApplicationTerminalTab
						{organizationId}
						{projectId}
						environmentId={environment.id}
						{applicationId}
						hostServerName={hostServer?.name}
						onOpenHostShell={hostServer ? () => (hostShellOpen = true) : undefined} />
					<ServerTerminalDialog
						open={hostShellOpen}
						{organizationId}
						server={hostServer}
						onOpenChange={(next) => (hostShellOpen = next)} />
				{:else if activeTab === 'metrics'}
					<ApplicationMetricsTab
						organizationId={organizationId ?? ''}
						{projectId}
						environmentId={environment.id}
						{applicationId}
						containerNames={metricsContainerNames} />
				{:else if activeTab === 'analytics'}
					<AnalyticsPanel
						description={t('analytics.panel.app_description')}
						load={(range) =>
							getApplicationAnalytics(
								organizationId ?? '',
								projectId,
								environment!.id,
								applicationId,
								range,
							)} />
				{:else if activeTab === 'tasks'}
					<ApplicationTasksTab
						organizationId={organizationId ?? ''}
						{projectId}
						environmentId={environment.id}
						{applicationId}
						applicationName={application.name} />
				{:else if activeTab === 'logs'}
					<ApplicationLogsTab
						{organizationId}
						{projectId}
						environmentId={environment.id}
						{applicationId}
						onAskAssistant={(text) =>
							askAboutLogs(
								projectDetail?.project.name ?? '',
								application?.name ?? '',
								'runtime',
								text,
							)} />
				{/if}
			</div>
		{/if}
	{/if}
</div>

<RedeployConfirmDialog bind:open={redeployOpen} port={redeployPort} ondeploy={deploy} />
<RedeployWithPullDialog
	bind:open={redeployWithPullOpen}
	applicationName={application?.name ?? ''}
	ondeploy={runComposeDeploy} />
<ApplicationFormDialog
	bind:open={editApplicationOpen}
	{application}
	onSubmit={handleEditApplication}
	onUploadIcon={handleUploadApplicationIcon} />
<DeleteResourceDialog
	bind:open={deleteDialogOpen}
	resourceName={application?.name ?? ''}
	resourceType="application"
	ondelete={handleDeleteApplication} />
