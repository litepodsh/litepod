<script lang="ts">
	import { allowedHere } from '#lib/state/permissions.svelte.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Label } from '#lib/components/ui/label/index.js';
	import SearchableDropdown from '#lib/components/custom/searchable-dropdown.svelte';
	import GitProviderIcon from '#lib/components/custom/git-provider-icon.svelte';
	import BoxIcon from '@lucide/svelte/icons/box';
	import GitBranchIcon from '@lucide/svelte/icons/git-branch';
	import FileCode2Icon from '@lucide/svelte/icons/file-code-2';
	import ContainerIcon from '@lucide/svelte/icons/container';
	import type {
		ContainerStatusDto,
		DeployHookDto,
		DeploymentDto,
		PortMapping,
		SourceConfigDto,
	} from '#lib/clients/project-client.js';
	import SoftSwitch from '#lib/components/custom/soft-switch.svelte';
	import * as AlertDialog from '#lib/components/ui/alert-dialog/index.js';
	import DeleteConfirmControl from '#lib/components/custom/delete-confirm-control.svelte';
	import * as Tooltip from '#lib/components/ui/tooltip/index.js';
	import type { Component } from 'svelte';
	import CopyButton from '#lib/components/custom/copy-button.svelte';
	import OsIcon from '#lib/components/os-icon.svelte';
	import RefreshCwIcon from '@lucide/svelte/icons/refresh-cw';
	import RocketIcon from '@lucide/svelte/icons/rocket';
	import PlayIcon from '@lucide/svelte/icons/play';
	import SquareIcon from '@lucide/svelte/icons/square';
	import Trash2Icon from '@lucide/svelte/icons/trash-2';
	import ShieldCheckIcon from '@lucide/svelte/icons/shield-check';
	import type { GitProviderDto, GitRepositoryDto } from '#lib/clients/git-provider-client.js';
	import { toast } from '#lib/utils/toast.js';
	import { t } from '#lib/i18n/index.svelte.js';
	import ApplicationComposeTab from '#lib/components/projects/application-compose-tab.svelte';
	import AnimatedFileUpload from '#lib/components/custom/animated-file-upload.svelte';
	import { untrack } from 'svelte';
	import { detectArtifactFileKind } from '#lib/utils/artifact-file.js';
	import { deployLabel } from '#lib/utils/deploy-label.js';
	import { isRunning } from '#lib/utils/container-status.js';
	import type {
		ComposeContent,
		ComposeServiceStatus,
		ValidationResponse,
	} from '#lib/clients/project-client.js';
	import {
		formatCpu,
		formatMemory,
		type ContainerMetric,
	} from '#lib/clients/container-metrics-client.js';

	let {
		deployment,
		source,
		providers,
		deployHook = null,
		busy = false,
		containerStatus = null,
		containerMetrics = new Map(),
		onUpload,
		onRemoveArtifact,
		onDeploy,
		onStop,
		onStart,
		onRemoveRuntime,
		onSaveSource,
		onSetCleanBuild,
		onLoadBranches,
		onLoadPublicBranches,
		onLoadRepositories,
		onRotateDeployHook,
		composeContent = '',
		composePortMappings = [],
		composeStatus = [],
		runtimeVariableNames = [],
		onComposeSave,
		onComposeDeploy,
		onComposeValidate,
		onComposeStop,
		onComposeStart,
		onComposeContentChange,
		onCreateRuntimeVariables,
		templateDocsUrl = null,
	}: {
		deployment: DeploymentDto;
		source: SourceConfigDto;
		providers: GitProviderDto[];
		deployHook?: DeployHookDto | null;
		busy?: boolean;
		containerStatus?: ContainerStatusDto | null;
		containerMetrics?: ReadonlyMap<string, ContainerMetric>;
		onUpload: (file: File) => Promise<void>;
		onRemoveArtifact: () => Promise<void>;
		onDeploy: (cleanBuild?: boolean) => Promise<void>;
		onStop: () => Promise<void>;
		onStart: () => Promise<void>;
		onRemoveRuntime: () => Promise<void>;
		onSaveSource: (input: SourceConfigDto) => Promise<void>;
		onSetCleanBuild: (value: boolean) => Promise<void>;
		onLoadBranches: (providerId: string, repoUrl: string) => Promise<string[]>;
		/** Branch listing with no saved provider — a public repo URL, or one carrying
		 * `user:token@host` credentials. */
		onLoadPublicBranches: (repoUrl: string) => Promise<string[]>;
		onLoadRepositories: (providerId: string) => Promise<GitRepositoryDto[]>;
		onRotateDeployHook: () => Promise<void>;
		composeContent?: string;
		composePortMappings?: PortMapping[];
		composeStatus?: ComposeServiceStatus[];
		runtimeVariableNames?: string[];
		onComposeSave: (content: string, portMappings: PortMapping[]) => Promise<ComposeContent>;
		onComposeDeploy: () => Promise<void>;
		onComposeValidate: (content: string) => Promise<ValidationResponse>;
		onComposeStop: () => Promise<void>;
		onComposeStart: () => Promise<void>;
		onComposeContentChange: (content: string) => void;
		onCreateRuntimeVariables: (created: { name: string; value: string }[]) => Promise<void>;
		/** Catalog setup page, when the application was created from a template. */
		templateDocsUrl?: string | null;
	} = $props();

	let deployType = $state<'static' | 'git' | 'compose' | 'image'>(
		untrack(() => source.deploy_type),
	);
	$effect(() => {
		deployType = source.deploy_type;
	});
	let buildMethod = $state<'docker' | 'compose' | 'railpack'>(untrack(() => source.build_method));
	$effect(() => {
		buildMethod = source.build_method;
	});
	let providerId = $state<string | null>(untrack(() => source.git_provider_id));
	$effect(() => {
		providerId = source.git_provider_id;
	});
	let repoUrl = $state(untrack(() => source.git_url ?? ''));
	$effect(() => {
		repoUrl = source.git_url ?? '';
	});
	let branch = $state<string | null>(untrack(() => source.git_branch));
	$effect(() => {
		branch = source.git_branch;
	});
	let dockerfilePath = $state(untrack(() => source.dockerfile_path ?? './Dockerfile'));
	$effect(() => {
		dockerfilePath = source.dockerfile_path ?? './Dockerfile';
	});
	let composeFilePath = $state(untrack(() => source.compose_file_path ?? './docker-compose.yml'));
	$effect(() => {
		composeFilePath = source.compose_file_path ?? './docker-compose.yml';
	});
	let composePublicService = $state(untrack(() => source.compose_public_service ?? ''));
	$effect(() => {
		composePublicService = source.compose_public_service ?? '';
	});
	let railpackProjectPath = $state(untrack(() => source.railpack_project_path ?? '.'));
	$effect(() => {
		railpackProjectPath = source.railpack_project_path ?? '.';
	});
	let railpackStartCommand = $state(untrack(() => source.railpack_start_command ?? ''));
	$effect(() => {
		railpackStartCommand = source.railpack_start_command ?? '';
	});
	let containerPort = $state(untrack(() => String(source.container_port ?? 3000)));
	$effect(() => {
		containerPort = String(source.container_port ?? 3000);
	});
	let autoDeploy = $state(untrack(() => source.auto_deploy));
	$effect(() => {
		autoDeploy = source.auto_deploy;
	});
	let watchPaths = $state(untrack(() => source.watch_paths ?? ''));
	$effect(() => {
		watchPaths = source.watch_paths ?? '';
	});
	let imageUrl = $state(untrack(() => source.image_url ?? ''));
	$effect(() => {
		imageUrl = source.image_url ?? '';
	});
	let registryUsername = $state('');
	let registryPassword = $state('');
	let restartPolicy = $state<'no' | 'always' | 'unless-stopped' | 'on-failure'>(
		untrack(() => {
			const current = source.restart_policy as
				| 'no'
				| 'always'
				| 'unless-stopped'
				| 'on-failure'
				| undefined;
			return current === 'no' || current === 'always' || current === 'on-failure'
				? current
				: 'unless-stopped';
		}),
	);
	$effect(() => {
		const current = source.restart_policy as
			| 'no'
			| 'always'
			| 'unless-stopped'
			| 'on-failure'
			| undefined;
		if (current === 'no' || current === 'always' || current === 'on-failure') {
			restartPolicy = current;
		} else {
			restartPolicy = 'unless-stopped';
		}
	});
	let cleanBuild = $state(untrack(() => source.clean_build));
	$effect(() => {
		cleanBuild = source.clean_build;
	});
	let branches = $state<string[]>([]);
	let loadingBranches = $state(false);
	let repositories = $state<GitRepositoryDto[]>([]);
	let loadingRepositories = $state(false);
	let removeRuntimeOpen = $state(false);
	let loadedProviderId: string | null = null;
	let branchedRepoUrl: string | null = null;
	// Deliberate one-shot snapshot of repoUrl at mount, used to detect whether
	// the user switched to a different repo (see $effect that loads branches).
	// svelte-ignore state_referenced_locally
	const initialRepoUrl = repoUrl;
	const composeIsRunning = $derived(
		composeStatus.some((service) =>
			['running', 'up'].some((value) => service.status.toLowerCase().startsWith(value)),
		),
	);
	const containerIsRunning = $derived(isRunning(containerStatus?.status ?? ''));
	const runningComposeServices = $derived(
		composeStatus.filter((service) =>
			['running', 'up'].some((value) => service.status.toLowerCase().startsWith(value)),
		),
	);
	const visibleStatus = $derived(
		deployType === 'compose' && composeStatus.length > 0
			? composeIsRunning
				? 'running'
				: 'stopped'
			: (containerStatus?.label ?? deployment.status.replaceAll('_', ' ')),
	);

	let providerItems = $derived(
		providers.map((provider) => ({
			value: provider.id,
			label: provider.name,
			description: provider.host_url || provider.provider_type,
			icon: provider.provider_type,
		})),
	);
	let repositoryItems = $derived(
		repositories.map((repository) => ({
			value: repository.clone_url,
			label: repository.full_name,
			description: repository.private
				? t('projects.application_general.repo_private')
				: t('projects.application_general.repo_public'),
		})),
	);
	let branchItems = $derived(branches.map((name) => ({ value: name, label: name })));
	let providerType = $derived(
		providers.find((provider) => provider.id === providerId)?.provider_type ?? null,
	);

	function statusClass(label: string): string {
		switch (label.toLowerCase()) {
			case 'running':
				return 'bg-success/15 text-success';
			// A container that died must not look like one that was never deployed.
			case 'stopped':
			case 'exited':
			case 'dead':
			case 'failed':
				return 'bg-destructive/15 text-destructive';
			case 'paused':
			// The server did not answer, so the container may well still be up.
			case 'unavailable':
				return 'bg-warning/15 text-warning';
			case 'created':
				return 'bg-info/15 text-info';
			default:
				return 'bg-muted text-muted-foreground';
		}
	}
	// Any provider type with a known push-webhook signature scheme can auto-deploy.
	// Generic providers (plain URL + token) have no webhook route.
	const WEBHOOK_PROVIDER_TYPES = ['github', 'gitea', 'forgejo', 'gitlab'];
	let canAutoDeploy = $derived(
		WEBHOOK_PROVIDER_TYPES.includes(
			providers.find((provider) => provider.id === providerId)?.provider_type ?? '',
		),
	);

	// Provider picked → pull every repository the token can see (organization/repo).
	async function loadRepositories(provider: string) {
		loadedProviderId = provider;
		loadingRepositories = true;
		repositories = [];
		try {
			const list = await onLoadRepositories(provider);
			if (loadedProviderId === provider) repositories = list;
		} finally {
			if (loadedProviderId === provider) loadingRepositories = false;
		}
	}

	$effect(() => {
		const provider = providerId;
		if (
			deployType !== 'git' ||
			!provider ||
			provider === loadedProviderId ||
			providerType === 'generic'
		)
			return;
		void loadRepositories(provider);
	});

	// Repository picked from the list → load its branches straight away.
	$effect(() => {
		const url = repoUrl.trim();
		if (!providerId || !url || url === branchedRepoUrl) return;
		const repository = repositories.find((item) => item.clone_url === url);
		if (!repository) return;
		branchedRepoUrl = url;
		// Keep the saved branch on first load for the already-configured repo; only default
		// to the repo's main branch when the user actually switches to a different repository.
		const keepSavedBranch = url === initialRepoUrl && !!branch;
		if (repository.default_branch && !keepSavedBranch) branch = repository.default_branch;
		void loadBranches();
	});
	let gitReady = $derived(
		!!(
			repoUrl.trim() &&
			branch &&
			Number(containerPort) > 0 &&
			(buildMethod === 'railpack' ||
				(buildMethod === 'docker' && dockerfilePath.trim()) ||
				(buildMethod === 'compose' && composeFilePath.trim() && composePublicService.trim()))
		),
	);
	let canDeploy = $derived(
		deployType === 'static'
			? !!deployment.artifact
			: deployType === 'image'
				? !!imageUrl.trim() && Number(containerPort) > 0
				: gitReady,
	);

	async function validateArtifactFile(file: File) {
		try {
			const kind = await detectArtifactFileKind(file);
			if (!kind) {
				toast.error(t('projects.application_general.invalid_artifact_file'));
				return null;
			}
			return { file, kind };
		} catch {
			toast.error(t('projects.application_general.unreadable_file'));
			return null;
		}
	}

	async function loadBranches() {
		if (!repoUrl.trim()) {
			toast.error(t('projects.application_general.repo_url_required'));
			return;
		}
		loadingBranches = true;
		try {
			branches = providerId
				? await onLoadBranches(providerId, repoUrl.trim())
				: await onLoadPublicBranches(repoUrl.trim());
			if (branch && !branches.includes(branch)) branch = null;
			if (branches.length === 0) toast.error(t('projects.application_general.no_branches_found'));
		} finally {
			loadingBranches = false;
		}
	}

	async function selectBranch(value: string) {
		branch = value;
		await save();
	}

	async function save() {
		const port = Number(containerPort);
		if (
			(deployType === 'git' || deployType === 'image') &&
			(!Number.isInteger(port) || port < 1 || port > 65535)
		) {
			toast.error(t('projects.application_general.container_port_range'));
			return;
		}
		await onSaveSource({
			deploy_type: deployType,
			build_method:
				deployType === 'compose' ? 'compose' : deployType === 'git' ? buildMethod : 'docker',
			git_provider_id: deployType === 'git' ? providerId : null,
			git_url: deployType === 'git' ? repoUrl.trim() : null,
			git_branch: deployType === 'git' ? branch : null,
			dockerfile_path:
				deployType === 'git' && buildMethod === 'docker' ? dockerfilePath.trim() : null,
			compose_file_path:
				deployType === 'git' && buildMethod === 'compose' ? composeFilePath.trim() : null,
			compose_public_service:
				deployType === 'git' && buildMethod === 'compose' ? composePublicService.trim() : null,
			container_port: deployType === 'git' || deployType === 'image' ? port : null,
			railpack_project_path:
				deployType === 'git' && buildMethod === 'railpack'
					? railpackProjectPath.trim() || '.'
					: '.',
			railpack_start_command:
				deployType === 'git' && buildMethod === 'railpack'
					? railpackStartCommand.trim() || null
					: source.railpack_start_command,
			auto_deploy: deployType === 'git' && canAutoDeploy && autoDeploy,
			watch_paths:
				deployType === 'git' && canAutoDeploy && autoDeploy ? watchPaths.trim() || null : null,
			// Registry-image source. An empty password keeps the saved one when the
			// image reference is unchanged; blanking it out stores nothing.
			image_url: deployType === 'image' ? imageUrl.trim() || null : null,
			registry_username: deployType === 'image' ? registryUsername.trim() || null : null,
			registry_password: deployType === 'image' ? registryPassword || null : null,
			restart_policy: deployType === 'image' ? restartPolicy : null,
			clean_build: cleanBuild,
			// Resource limits apply to every deploy type (build and runtime share
			// one limit now), so they survive a source change.
			build_cpu_cores: source.build_cpu_cores,
			build_memory_mib: source.build_memory_mib,
			build_swap_mib: source.build_swap_mib,
			// Read-only, derived by the backend from what's actually stored — sent back
			// unchanged since the type is shared between the source response and this save.
			has_git_credentials: source.has_git_credentials,
			has_registry_credentials: source.has_registry_credentials,
		});
	}

	async function saveCompose(content: string, portMappings: PortMapping[]) {
		await save();
		return onComposeSave(content, portMappings);
	}

	async function toggleCleanBuild(value: boolean) {
		const previous = cleanBuild;
		cleanBuild = value;
		try {
			await onSetCleanBuild(value);
		} catch {
			cleanBuild = previous;
		}
	}

	async function removeRuntime() {
		await onRemoveRuntime();
		removeRuntimeOpen = false;
	}
</script>

<!--
	Deploy, Start, Stop and Delete all look alike but differ in what survives
	them, so each one carries a tooltip spelling out its effect. The snippet
	keeps the tooltip wiring in one place instead of eight nested blocks.
-->
{#snippet runtimeAction({
	tip,
	icon: Icon,
	label,
	onclick,
	variant = 'default',
	disabled = false,
}: {
	tip: string;
	icon: Component<{ class?: string }>;
	label: string;
	onclick: () => void;
	variant?: 'default' | 'outline' | 'destructive';
	disabled?: boolean;
})}
	<Tooltip.Root delayDuration={200}>
		<Tooltip.Trigger>
			{#snippet child({ props })}
				<Button {...props} {variant} {disabled} {onclick} aria-label={label}>
					<Icon class="size-4" />
					<span class="hidden sm:inline">{label}</span>
				</Button>
			{/snippet}
		</Tooltip.Trigger>
		<Tooltip.Content class="max-w-xs">{tip}</Tooltip.Content>
	</Tooltip.Root>
{/snippet}

<div class="grid min-w-0 gap-6">
	<section
		class="order-first lp-lifted min-w-0 rounded-[min(var(--radius-4xl),28px)] bg-card px-5 py-4 ring-1 ring-foreground/8">
		<div class="flex flex-wrap items-center justify-between gap-3">
			<div class="flex min-w-0 flex-wrap items-center gap-2">
				<p class="text-sm font-medium">{t('projects.application_general.status')}</p>
				<span
					class="rounded-full px-2 py-0.5 text-xs font-medium {statusClass(visibleStatus)}"
					title={containerStatus?.status === 'unavailable'
						? t('projects.application_general.server_unavailable_title')
						: undefined}>
					{visibleStatus}
				</span>
				{#if deployType === 'compose'}
					<span class="text-xs text-muted-foreground"
						>{t('projects.application_general.containers_running', {
							running: runningComposeServices.length,
							total: composeStatus.length,
						})}</span>
				{/if}
			</div>
			{#if deployType !== 'compose'}
				<div class="flex flex-wrap items-center gap-2">
					{#if deployType === 'git' && buildMethod === 'railpack'}
						<!-- <label class="flex items-center gap-2 text-sm text-muted-foreground">
							<SoftSwitch
								checked={cleanBuild}
								onCheckedChange={(value) => void toggleCleanBuild(value)}
								disabled={busy || !canDeploy}
							/>
							<span>Clean build</span>
						</label> -->
					{/if}
					{#if allowedHere('app.deploy')}
						{@render runtimeAction({
							tip: containerIsRunning
								? t('projects.application_general.redeploy_tip')
								: t('projects.application_general.deploy_tip'),
							icon: RocketIcon,
							label: deployLabel(containerIsRunning),
							onclick: () => onDeploy(cleanBuild),
							disabled: busy || !canDeploy,
						})}
						{#if containerStatus?.label === 'Running'}
							{@render runtimeAction({
								tip: t('projects.application_general.stop_tip'),
								icon: SquareIcon,
								label: t('projects.environment.stop'),
								onclick: onStop,
								variant: 'outline',
								disabled: busy,
							})}
						{:else if containerStatus?.label === 'Stopped' || containerStatus?.label === 'Paused' || containerStatus?.label === 'Created'}
							{@render runtimeAction({
								tip: t('projects.application_general.start_tip'),
								icon: PlayIcon,
								label: t('projects.environment.start'),
								onclick: onStart,
								variant: 'outline',
								disabled: busy,
							})}
						{/if}
					{/if}
					{@render runtimeAction({
						tip: t('projects.application_general.delete_tip'),
						icon: Trash2Icon,
						label: t('projects.common.delete'),
						onclick: () => {
							removeRuntimeOpen = true;
						},
						variant: 'destructive',
						disabled: busy,
					})}
				</div>
			{:else}
				<div class="flex flex-wrap gap-2">
					{#if allowedHere('app.deploy')}
						{@render runtimeAction({
							tip: composeIsRunning
								? t('projects.application_general.compose_redeploy_tip')
								: t('projects.application_general.compose_deploy_tip'),
							icon: RocketIcon,
							label: deployLabel(composeIsRunning),
							onclick: onComposeDeploy,
							disabled: busy,
						})}
						{#if composeIsRunning}
							{@render runtimeAction({
								tip: t('projects.application_general.compose_stop_tip'),
								icon: SquareIcon,
								label: t('projects.environment.stop'),
								onclick: onComposeStop,
								variant: 'outline',
								disabled: busy,
							})}
						{:else}
							{@render runtimeAction({
								tip: t('projects.application_general.compose_start_tip'),
								icon: PlayIcon,
								label: t('projects.environment.start'),
								onclick: onComposeStart,
								variant: 'outline',
								disabled: busy,
							})}
						{/if}
					{/if}
					{@render runtimeAction({
						tip: t('projects.application_general.compose_delete_tip'),
						icon: Trash2Icon,
						label: t('projects.common.delete'),
						onclick: () => {
							removeRuntimeOpen = true;
						},
						variant: 'destructive',
						disabled: busy,
					})}
				</div>
			{/if}
		</div>
		<div class="mt-3 flex flex-wrap gap-2">
			{#if deployType === 'compose'}
				{#if composeStatus.length === 0}
					<span class="text-xs text-muted-foreground"
						>{t('projects.application_general.no_compose_containers')}</span>
				{:else}
					{#each composeStatus as service}
						{@const serviceRunning = ['running', 'up'].some((value) =>
							service.status.toLowerCase().startsWith(value),
						)}
						{@const metric = containerMetrics.get(service.container_name)}
						<span
							class="inline-flex max-w-full items-center gap-1.5 rounded-md border px-2 py-1 text-xs {serviceRunning
								? 'border-success/30 bg-success/10 text-success'
								: 'bg-muted text-muted-foreground'}">
							<span
								class="size-1.5 shrink-0 rounded-full {serviceRunning
									? 'bg-success'
									: 'bg-muted-foreground'}"></span>
							<span class="shrink-0 font-medium">{service.service}</span>
							<span class="min-w-0 truncate font-mono">{service.container_name}</span>
							{#if serviceRunning}<span class="text-muted-foreground"
									>{metric
										? `CPU ${formatCpu(metric.cpu_percent)} · RAM ${formatMemory(metric.memory_usage_bytes, metric.memory_limit_bytes)}`
										: t('projects.application_general.metrics_unavailable')}</span
								>{/if}
						</span>
					{/each}
				{/if}
			{:else if containerStatus?.container_name}
				{@const containerRunning = containerStatus.label === 'Running'}
				{@const metric = containerMetrics.get(containerStatus.container_name)}
				<span
					class="inline-flex max-w-full items-center gap-1.5 rounded-md border px-2 py-1 text-xs {containerRunning
						? 'border-success/30 bg-success/10 text-success'
						: 'bg-muted text-muted-foreground'}">
					<span
						class="size-1.5 shrink-0 rounded-full {containerRunning
							? 'bg-success'
							: 'bg-muted-foreground'}"></span>
					<span class="min-w-0 truncate font-mono">{containerStatus.container_name}</span>
					<span class="shrink-0">{containerStatus.status}</span>
					{#if containerRunning}<span class="text-muted-foreground"
							>{metric
								? `CPU ${formatCpu(metric.cpu_percent)} · RAM ${formatMemory(metric.memory_usage_bytes, metric.memory_limit_bytes)}`
								: t('projects.application_general.metrics_unavailable')}</span
						>{/if}
				</span>
			{:else}
				<span class="text-xs text-muted-foreground"
					>{t('projects.application_general.no_container_deployed')}</span>
			{/if}
		</div>
		{#if deployment.replicas.length > 0}
			{@const runningReplicas = deployment.replicas.filter(
				(replica) => replica.status === 'running',
			).length}
			<div class="mt-3 rounded-lg border p-3">
				<div class="flex flex-wrap items-center gap-2">
					<p class="text-xs font-medium">{t('projects.application_general.replicas')}</p>
					<span class="text-xs text-muted-foreground">
						{t('projects.application_general.replicas_summary', {
							running: runningReplicas,
							total: deployment.replicas.length,
						})}
					</span>
				</div>
				<ul class="mt-2 space-y-1.5">
					{#each deployment.replicas as replica (replica.server_id)}
						<li class="flex items-center justify-between gap-3 text-sm">
							<span class="flex min-w-0 items-center gap-2">
								<span class="shrink-0 text-muted-foreground"
									><OsIcon
										os={replica.os}
										distribution={replica.distribution}
										class="size-4" /></span>
								<span class="truncate font-medium">{replica.server_name}</span>
								{#if replica.address}
									<span class="truncate font-mono text-xs text-muted-foreground"
										>{replica.address}</span>
								{/if}
							</span>
							<span
								class="shrink-0 rounded-full px-2 py-0.5 text-xs font-medium {statusClass(
									replica.status,
								)}">
								{replica.status.replaceAll('_', ' ')}
							</span>
						</li>
					{/each}
				</ul>
			</div>
		{/if}
		{#if deployment.public_url}<div
				class="mt-3 flex flex-wrap items-center gap-x-2 gap-y-1 text-xs">
				<span class="text-muted-foreground">{t('projects.application_general.public_url')}</span><a
					class="break-all font-medium underline-offset-4 hover:underline"
					href={deployment.public_url}
					target="_blank"
					rel="noreferrer">{deployment.public_url}</a>
			</div>{:else}<p class="mt-3 text-xs text-muted-foreground">
				{t('projects.application_general.no_domain_yet')}
			</p>{/if}
		{#if deployHook}
			<div class="mt-3 mb-1 grid gap-2 rounded-lg bg-muted/50 p-2.5 text-sm">
				<p class="text-xs text-muted-foreground">{t('projects.application_general.deploy_url')}</p>
				<div class="flex flex-wrap gap-2">
					<Input
						readonly
						value={deployHook.url}
						class="font-mono text-xs"
						aria-label={t('projects.application_general.deploy_url')} />
					<CopyButton
						value={deployHook.url}
						label={null}
						revealOnHover={false}
						errorMessage={t('projects.application_general.copy_deploy_url_failed')}
						class="inline-flex size-9 shrink-0 items-center justify-center rounded-md border bg-background hover:bg-accent hover:text-accent-foreground"
						iconClass="size-4" />
					<Button
						variant="outline"
						size="icon"
						disabled={busy}
						onclick={() => void onRotateDeployHook()}
						aria-label={t('projects.application_general.rotate_deploy_url')}
						><RefreshCwIcon class="size-4" /></Button>
				</div>
				<p class="text-xs text-muted-foreground">
					{t('projects.application_general.deploy_hook_hint')}
				</p>
				{#if deployType === 'image'}
					<p class="mt-1 text-xs text-muted-foreground">
						{t('projects.application_general.deploy_hook_docker_hub_hint')}
					</p>
				{/if}
			</div>
		{/if}
	</section>

	<section
		class="lp-lifted min-w-0 rounded-[min(var(--radius-4xl),28px)] bg-card p-5 ring-1 ring-foreground/8">
		<p class="text-base font-semibold tracking-tight">{t('projects.application_general.source')}</p>
		<div class="mt-3 flex flex-wrap gap-2">
			<Button
				variant={deployType === 'static' ? 'default' : 'outline'}
				disabled={busy}
				onclick={() => (deployType = 'static')}
				><BoxIcon class="size-4" /> {t('projects.application_general.static_files')}</Button>
			<Button
				variant={deployType === 'git' ? 'default' : 'outline'}
				disabled={busy}
				onclick={() => (deployType = 'git')}
				><GitBranchIcon class="size-4" />
				{t('projects.application_general.git_repository')}</Button>
			<Button
				variant={deployType === 'compose' ? 'default' : 'outline'}
				disabled={busy}
				onclick={() => (deployType = 'compose')}
				><FileCode2Icon class="size-4" /> {t('projects.application_general.compose')}</Button>
			<Button
				variant={deployType === 'image' ? 'default' : 'outline'}
				disabled={busy}
				onclick={() => (deployType = 'image')}
				><ContainerIcon class="size-4" />
				{t('projects.application_general.registry_image')}</Button>
		</div>
		{#if deployType === 'static' && source.deploy_type === 'git'}
			<div class="mt-4">
				<Button variant="outline" disabled={busy} onclick={save}
					>{t('projects.application_general.save_as_static')}</Button>
			</div>
		{/if}
		{#if deployType === 'static'}
			<div class="mt-5">
				<AnimatedFileUpload
					accept=".zip,.html,.htm,application/zip,text/html"
					heading={t('projects.application_general.artifact_heading')}
					description={t('projects.application_general.artifact_description')}
					validate={validateArtifactFile}
					onMultipleFiles={() => toast.error(t('projects.application_general.one_file_at_a_time'))}
					{onUpload}
					onDelete={onRemoveArtifact}
					currentFile={deployment.artifact
						? {
								name: deployment.artifact.filename,
								size: deployment.artifact.byte_size,
								kind: deployment.artifact.kind,
							}
						: null} />
			</div>
		{:else if deployType === 'git'}
			<div class="mt-5 grid gap-4">
				{#if providers.length === 0}
					<p class="text-sm text-muted-foreground">
						{t('projects.application_general.no_git_providers')}
						<a
							class="font-medium text-foreground underline-offset-4 hover:underline"
							href="/git-providers">{t('projects.application_general.add_one_first')}</a
						>.
					</p>
				{/if}
				<div class="grid gap-2">
					<Label
						>{t('projects.application_general.git_provider')}
						<span class="text-muted-foreground"
							>{t('projects.application_general.git_provider_optional_hint')}</span
						></Label>
					<SearchableDropdown
						items={providerItems}
						bind:selectedValue={providerId}
						label={t('projects.application_general.choose_provider')}
						disabled={busy || providers.length === 0}>
						{#snippet icon(type)}
							<GitProviderIcon {type} class="size-4 shrink-0" />
						{/snippet}
					</SearchableDropdown>
				</div>
				{#if providerId && providerType !== 'generic'}
					<div class="grid gap-2">
						<Label>{t('projects.application_general.repository')}</Label>
						<div class="flex gap-2">
							<SearchableDropdown
								items={repositoryItems}
								selectedValue={repositories.some((item) => item.clone_url === repoUrl.trim())
									? repoUrl.trim()
									: null}
								onSelect={(value) => (repoUrl = value)}
								label={loadingRepositories
									? t('projects.application_general.loading_repositories')
									: repositories.length
										? t('projects.application_general.choose_repository')
										: t('projects.application_general.no_repositories_found')}
								searchPlaceholder={t('projects.application_general.search_repos_placeholder')}
								disabled={busy || loadingRepositories || repositories.length === 0} />
							<Button
								variant="ghost"
								disabled={busy || loadingRepositories}
								onclick={() => providerId && loadRepositories(providerId)}
								>{t('projects.application_general.refresh')}</Button>
						</div>
						<p class="text-xs text-muted-foreground">
							{t('projects.application_general.repositories_hint')} <code>organization/repo</code>.
						</p>
					</div>
				{/if}
				<div class="grid gap-2">
					<Label for="repo-url">{t('projects.application_general.repository_url')}</Label>
					<div class="flex gap-2">
						<Input
							id="repo-url"
							bind:value={repoUrl}
							placeholder="https://github.com/octocat/hello-world.git"
							disabled={busy} />
						<Button
							variant="outline"
							disabled={busy || loadingBranches || !repoUrl.trim()}
							onclick={loadBranches}
							>{loadingBranches
								? t('projects.application_general.loading')
								: t('projects.application_general.load_branches')}</Button>
					</div>
					{#if !providerId}
						<p class="text-xs text-muted-foreground">
							{t('projects.application_general.no_provider_hint')}<code
								>https://user:token@host/owner/repo.git</code
							>{t('projects.application_general.private_clone_hint')}
							{#if source.has_git_credentials}
								{t('projects.application_general.saved_credentials_hint')}
								<code>user:token@…</code>
								{t('projects.application_general.credentials_replace_hint')}
							{/if}
						</p>
					{/if}
				</div>
				<div class="grid gap-2">
					<Label>{t('projects.application_general.branch')}</Label>
					<SearchableDropdown
						items={branchItems}
						selectedValue={branch}
						onSelect={(value) => void selectBranch(value)}
						label={branches.length
							? t('projects.application_general.choose_branch')
							: t('projects.application_general.load_branches_first')}
						disabled={busy || branches.length === 0} />
				</div>
				<div>
					<Button variant="outline" disabled={busy} onclick={save}
						>{t('projects.application_general.save_source')}</Button>
				</div>
			</div>
		{:else if deployType === 'compose'}
			<div class="mt-5">
				<ApplicationComposeTab
					{busy}
					{composeContent}
					{composePortMappings}
					{runtimeVariableNames}
					onSave={saveCompose}
					onDeploy={onComposeDeploy}
					onValidate={onComposeValidate}
					onContentChange={onComposeContentChange}
					running={composeIsRunning}
					{onCreateRuntimeVariables}
					{templateDocsUrl}
					showPortMappings={false} />
			</div>
		{:else if deployType === 'image'}
			<div class="mt-5 grid gap-4">
				<div class="grid gap-2">
					<Label for="image-url">{t('projects.application_general.image_reference')}</Label>
					<Input
						id="image-url"
						bind:value={imageUrl}
						placeholder="ghcr.io/owner/app:tag"
						disabled={busy} />
					<p class="text-xs text-muted-foreground">
						{t('projects.application_general.image_reference_hint')}
					</p>
				</div>
				<div class="grid gap-2">
					<Label for="image-port">{t('projects.application_general.container_port')}</Label>
					<Input
						id="image-port"
						type="number"
						min="1"
						max="65535"
						bind:value={containerPort}
						disabled={busy} />
					<p class="text-xs text-muted-foreground">
						{t('projects.application_general.container_port_hint')}
					</p>
				</div>
				<div class="grid gap-2">
					<Label for="restart-policy">{t('projects.application_general.restart_policy')}</Label>
					<select
						id="restart-policy"
						bind:value={restartPolicy}
						disabled={busy}
						class="flex h-9 w-full rounded-md border border-input bg-background px-3 py-1 text-sm shadow-sm transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50">
						<option value="unless-stopped"
							>{t('projects.application_general.restart_policy_unless_stopped')}</option>
						<option value="always"
							>{t('projects.application_general.restart_policy_always')}</option>
						<option value="on-failure"
							>{t('projects.application_general.restart_policy_on_failure')}</option>
						<option value="no">{t('projects.application_general.restart_policy_no')}</option>
					</select>
					<p class="text-xs text-muted-foreground">
						{t('projects.application_general.restart_policy_hint')}
					</p>
				</div>
				<div class="grid gap-2 rounded-lg border p-3">
					<p class="text-sm font-medium">
						{t('projects.application_general.registry_credentials')}
					</p>
					<p class="text-xs text-muted-foreground">
						{t('projects.application_general.registry_credentials_hint')}
						{#if source.has_registry_credentials}
							{t('projects.application_general.saved_registry_credentials_hint')}
						{/if}
					</p>
					<div class="grid gap-2 sm:grid-cols-2">
						<div class="grid gap-2">
							<Label for="registry-username"
								>{t('projects.application_general.registry_username')}</Label>
							<Input
								id="registry-username"
								bind:value={registryUsername}
								autocomplete="off"
								disabled={busy} />
						</div>
						<div class="grid gap-2">
							<Label for="registry-password"
								>{t('projects.application_general.registry_password')}</Label>
							<Input
								id="registry-password"
								type="password"
								bind:value={registryPassword}
								autocomplete="new-password"
								disabled={busy} />
						</div>
					</div>
				</div>
				<div>
					<Button variant="outline" disabled={busy} onclick={save}
						>{t('projects.application_general.save_source')}</Button>
				</div>
			</div>
		{/if}
	</section>

	{#if deployType === 'git'}
		<section
			class="lp-lifted min-w-0 rounded-[min(var(--radius-4xl),28px)] bg-card p-5 ring-1 ring-foreground/8">
			<p class="text-base font-semibold tracking-tight">
				{t('projects.application_general.build')}
			</p>
			<div class="mt-5 grid gap-4">
				<div class="grid gap-2">
					<Label>{t('projects.application_general.build_method')}</Label>
					<div class="flex flex-wrap gap-2">
						<Button
							variant={buildMethod === 'docker' ? 'default' : 'outline'}
							disabled={busy}
							onclick={() => (buildMethod = 'docker')}
							>{t('projects.application_general.docker')}</Button
						><Button
							variant={buildMethod === 'compose' ? 'default' : 'outline'}
							disabled={busy}
							onclick={() => (buildMethod = 'compose')}
							>{t('projects.application_general.docker_compose')}</Button
						><Button
							variant={buildMethod === 'railpack' ? 'default' : 'outline'}
							disabled={busy}
							onclick={() => (buildMethod = 'railpack')}
							>{t('projects.application_general.railpack')}</Button>
					</div>
				</div>
				{#if buildMethod === 'docker'}
					<div class="grid gap-2">
						<Label for="dockerfile-path">{t('projects.application_general.dockerfile_path')}</Label>
						<Input
							id="dockerfile-path"
							bind:value={dockerfilePath}
							placeholder="./Dockerfile"
							disabled={busy} />
						<p class="text-xs text-muted-foreground">
							{t('projects.application_general.dockerfile_path_hint')}
							<code>docker/prod/Dockerfile</code>.
						</p>
					</div>
				{:else if buildMethod === 'compose'}
					<div class="grid gap-2">
						<Label for="compose-path">{t('projects.application_general.compose_file_path')}</Label
						><Input
							id="compose-path"
							bind:value={composeFilePath}
							placeholder="./docker-compose.yml"
							disabled={busy} />
					</div>
					<div class="grid gap-2">
						<Label for="compose-service">{t('projects.application_general.public_service')}</Label
						><Input
							id="compose-service"
							bind:value={composePublicService}
							placeholder="web"
							disabled={busy} />
					</div>
				{:else}
					<div class="grid gap-2">
						<Label for="railpack-project-path"
							>{t('projects.application_general.railpack_project_path')}</Label
						><Input
							id="railpack-project-path"
							bind:value={railpackProjectPath}
							placeholder="."
							disabled={busy} />
						<p class="text-xs text-muted-foreground">
							{t('projects.application_general.railpack_path_hint')} <code>apps/web</code>.
						</p>
					</div>
					<div class="grid gap-2">
						<Label for="railpack-start-command"
							>{t('projects.application_general.start_command')}</Label
						><Input
							id="railpack-start-command"
							bind:value={railpackStartCommand}
							placeholder="node ./dist/server/entry.mjs"
							disabled={busy} />
						<p class="text-xs text-muted-foreground">
							{t('projects.application_general.start_command_hint')}
						</p>
					</div>
				{/if}
				<div class="grid gap-2">
					<Label for="container-port">{t('projects.application_general.container_port')}</Label>
					<Input
						id="container-port"
						type="number"
						min="1"
						max="65535"
						bind:value={containerPort}
						disabled={busy} />
					<p class="text-xs text-muted-foreground">
						{t('projects.application_general.container_port_hint')}
					</p>
				</div>
				<div class="grid gap-2 rounded-lg border p-3">
					<div class="flex items-center justify-between gap-3">
						<div>
							<p class="text-sm font-medium">{t('projects.application_general.auto_deploy')}</p>
							<p class="text-xs text-muted-foreground">
								{#if canAutoDeploy}
									{t('projects.application_general.auto_deploy_hint')}
									<code>{branch ?? t('projects.application_general.selected_branch')}</code>.
								{:else}
									{t('projects.application_general.webhook_required_hint')}
									<a class="underline-offset-4 hover:underline" href="/git-providers"
										>{t('projects.application_general.git_providers_page')}</a>
									{t('projects.application_general.page_suffix')}
								{/if}
							</p>
						</div>
						<SoftSwitch
							bind:checked={autoDeploy}
							disabled={busy || !canAutoDeploy}
							aria-label={t('projects.application_general.auto_deploy')} />
					</div>
					{#if canAutoDeploy && autoDeploy}
						<div class="grid gap-2">
							<Label for="watch-paths">{t('projects.application_general.watch_paths')}</Label>
							<Input
								id="watch-paths"
								bind:value={watchPaths}
								placeholder="apps/web,packages/shared"
								disabled={busy} />
							<p class="text-xs text-muted-foreground">
								{t('projects.application_general.watch_paths_hint')}
							</p>
						</div>
					{/if}
				</div>
				<div>
					<Button variant="outline" disabled={busy} onclick={save}
						>{t('projects.application_general.save_build')}</Button>
				</div>
			</div>
		</section>
	{/if}
</div>

<AlertDialog.Root
	open={removeRuntimeOpen}
	onOpenChange={(open) => {
		if (!open && !busy) {
			removeRuntimeOpen = false;
		}
	}}>
	<AlertDialog.Content>
		<AlertDialog.Header>
			<AlertDialog.Title
				>{t('projects.application_general.delete_runtime_title')}</AlertDialog.Title>
			<AlertDialog.Description>
				{t('projects.application_general.delete_runtime_outro')}
			</AlertDialog.Description>
			<div class="mt-2 grid gap-2.5 text-sm sm:grid-cols-2">
				<div class="rounded-lg border border-destructive/25 bg-destructive/5 p-3">
					<p class="mb-1.5 flex items-center gap-1.5 font-medium text-destructive">
						<Trash2Icon class="size-3.5" />{t(
							'projects.application_general.delete_runtime_removes',
						)}
					</p>
					<ul class="list-disc space-y-0.5 pl-4 text-muted-foreground">
						{#if deployType === 'compose'}
							<li>{t('projects.application_general.delete_runtime_compose')}</li>
							<li>{t('projects.application_general.delete_runtime_compose_volumes')}</li>
						{:else}
							<li>{t('projects.application_general.delete_runtime_container')}</li>
							<li>{t('projects.application_general.delete_runtime_container_volumes')}</li>
						{/if}
					</ul>
				</div>
				<div class="rounded-lg border border-success/25 bg-success/5 p-3">
					<p class="mb-1.5 flex items-center gap-1.5 font-medium text-success">
						<ShieldCheckIcon class="size-3.5" />{t(
							'projects.application_general.delete_runtime_keeps',
						)}
					</p>
					<ul class="list-disc space-y-0.5 pl-4 text-muted-foreground">
						<li>{t('projects.application_general.delete_runtime_keeps_app')}</li>
						<li>{t('projects.application_general.delete_runtime_keeps_project')}</li>
					</ul>
				</div>
			</div>
		</AlertDialog.Header>
		<AlertDialog.Footer>
			<AlertDialog.Cancel disabled={busy}>{t('projects.projects.cancel')}</AlertDialog.Cancel>
			<DeleteConfirmControl
				label={t('projects.application_general.delete_runtime')}
				disabled={busy}
				onconfirm={removeRuntime} />
		</AlertDialog.Footer>
	</AlertDialog.Content>
</AlertDialog.Root>

<style>
	/* Long paths / credential URLs in inline <code> must wrap, not push the card wider. */
	code {
		overflow-wrap: anywhere;
	}
</style>
