<script lang="ts" module>
	import type {
		ScheduledTaskRunner,
		ScheduledTaskShell,
	} from '#lib/clients/scheduled-tasks-client.js';

	export type LockedApplication = {
		id: string;
		label: string;
		projectId: string;
		environmentId: string;
	};

	// "Insert example" scripts per runner and shell. Code, not UI copy.
	const EXAMPLES: Record<
		ScheduledTaskRunner,
		{ label: string; image?: string; script: Partial<Record<ScheduledTaskShell, string>> }
	> = {
		ephemeral: {
			label: 'example_backup',
			image: 'postgres:17-alpine',
			script: {
				sh: 'set -eu\npg_dump "$DATABASE_URL" | gzip > /backup/db-$(date +%F).sql.gz\nfind /backup -name "*.sql.gz" -mtime +14 -delete\necho "backup ok"',
				bash: 'set -euo pipefail\nfile="/backup/db-$(date +%F).sql.gz"\npg_dump "$DATABASE_URL" | gzip > "$file"\necho "wrote $(du -h "$file" | cut -f1) to $file"',
			},
		},
		application_exec: {
			label: 'example_cleanup',
			script: {
				sh: 'node scripts/cleanup.js --older-than 30d',
				bash: 'set -euo pipefail\nnode scripts/cleanup.js --older-than 30d',
			},
		},
		remote_shell: {
			label: 'example_prune',
			script: {
				sh: 'podman image prune -af --filter "until=168h"\npodman system df',
				bash: 'set -euo pipefail\npodman image prune -af --filter "until=168h"\npodman system df',
			},
		},
		litepod_api: {
			label: 'example_api',
			script: {
				sh: 'curl -fsS -X POST "$LITEPOD_API_URL/api/maintenance/prune" \\\n  -H "Authorization: Bearer $LITEPOD_TOKEN"',
			},
		},
	};

	const TIMEOUTS: [number, string][] = [
		[60, '1m'],
		[300, '5m'],
		[900, '15m'],
		[3600, '1h'],
		[21600, '6h'],
	];
</script>

<script lang="ts">
	import { tick, untrack } from 'svelte';
	import CalendarCheckIcon from '@lucide/svelte/icons/calendar-check';
	import CheckIcon from '@lucide/svelte/icons/check';
	import FlaskConicalIcon from '@lucide/svelte/icons/flask-conical';
	import SquareIcon from '@lucide/svelte/icons/square';
	import XIcon from '@lucide/svelte/icons/x';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Dialog from '#lib/components/ui/dialog/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Label } from '#lib/components/ui/label/index.js';
	import * as NativeSelect from '#lib/components/ui/native-select/index.js';
	import SoftSwitch from '#lib/components/custom/soft-switch.svelte';
	import { getMe } from '#lib/clients/auth-client.js';
	import { getSystemVersion } from '#lib/clients/system-client.js';
	import { listServers, type Server } from '#lib/clients/server-client.js';
	import {
		composeStatus,
		getProject,
		listProjects,
		type ComposeServiceStatus,
	} from '#lib/clients/project-client.js';
	import {
		createScheduledTask,
		streamScheduledTaskTest,
		updateScheduledTask,
		type ScheduledTask,
		type ScheduledTaskDetail,
		type ScheduledTaskTestEvent,
	} from '#lib/clients/scheduled-tasks-client.js';
	import { isRunning } from '#lib/utils/container-status.js';
	import { describeCron, nextRuns } from '#lib/utils/cron.js';
	import { relativeTime } from '#lib/utils/relative-time.js';
	import { serverGroups } from '#lib/utils/server-visibility.js';
	import { renderTerminalText } from '#lib/utils/terminal-text.js';
	import { toast } from '#lib/utils/toast.js';
	import { i18n, t } from '#lib/i18n/index.svelte.js';
	import { cn } from '#lib/utils.js';
	import RunnerCards, { RUNNER_ICON } from './runner-cards.svelte';
	import ScriptEditor from './script-editor.svelte';
	import ShellPicker from './shell-picker.svelte';

	let {
		open = $bindable(false),
		organizationId,
		editing = null,
		cloneFrom = null,
		lockedApplication = null,
		onSaved,
	}: {
		open?: boolean;
		organizationId: string;
		/** The saved task to edit; null creates a new one. */
		editing?: ScheduledTaskDetail | null;
		/** Prefills a new task with a copy of this one (ignored while editing). */
		cloneFrom?: ScheduledTaskDetail | null;
		/** Opened from an app: preselects application_exec and fixes the app. */
		lockedApplication?: LockedApplication | null;
		onSaved?: (task: ScheduledTask, created: boolean) => void;
	} = $props();

	const presets = [
		{ label: t('organizations.tasks.every_minute'), value: '* * * * *' },
		{ label: t('organizations.tasks.every_five_minutes'), value: '*/5 * * * *' },
		{ label: t('organizations.tasks.every_hour'), value: '0 * * * *' },
		{ label: t('organizations.tasks.daily'), value: '0 0 * * *' },
		{ label: t('organizations.tasks.sunday'), value: '0 0 * * 0' },
		{ label: t('organizations.tasks.monthly'), value: '0 0 1 * *' },
	];

	let name = $state('');
	let description = $state('');
	let cron = $state('0 0 * * *');
	let runner = $state<ScheduledTaskRunner>('ephemeral');
	// ponytail: official bash image (Alpine base, ~6 MB) ships both sh and bash, enough for scripts.
	const DEFAULT_EPHEMERAL_IMAGE = 'docker.io/library/bash:5';
	let image = $state(DEFAULT_EPHEMERAL_IMAGE);
	let applicationId = $state('');
	let composeService = $state('');
	let shell = $state<ScheduledTaskShell>('bash');
	let script = $state('echo "Hello World"');
	let timeout = $state(3600);
	let networkAccess = $state(false);
	let serverId = $state('local');
	let saving = $state(false);

	// who is asking and where tasks can run; loaded once, the first time the dialog opens
	let sysadmin = $state(false);
	let isCloud = $state(false);
	let servers = $state<Server[]>([]);
	let contextLoaded: Promise<void> | null = null;

	function loadContext() {
		contextLoaded ??= Promise.all([getMe(), getSystemVersion(), listServers(organizationId)])
			.then(([me, version, list]) => {
				sysadmin = me.is_sysadmin;
				isCloud = version.is_cloud;
				servers = list;
			})
			.catch((error) => {
				contextLoaded = null;
				toast.error(error instanceof Error ? error.message : t('organizations.tasks.load_failed'));
			});
		return contextLoaded;
	}

	const groups = $derived(serverGroups(servers, isCloud));
	const selectedServer = $derived(servers.find((candidate) => candidate.id === serverId) ?? null);
	const runners = $derived<ScheduledTaskRunner[]>([
		'ephemeral',
		'application_exec',
		'remote_shell',
		...(sysadmin && !isCloud ? (['litepod_api'] as const) : []),
	]);

	function defaultServerId(): string {
		if (isCloud) return groups.remote.find((server) => server.state === 'ready')?.id ?? '';
		return 'local';
	}

	function reset() {
		stopTest();
		testRun = null;
		const task = editing ?? cloneFrom;
		name =
			cloneFrom && !editing
				? t('organizations.task_ui.copy_name', { name: cloneFrom.name })
				: (task?.name ?? '');
		description = task?.description ?? '';
		cron = task?.cron_expression ?? '0 0 * * *';
		runner = task?.runner ?? (lockedApplication ? 'application_exec' : 'ephemeral');
		image = task?.image ?? DEFAULT_EPHEMERAL_IMAGE;
		applicationId = task?.application_id ?? lockedApplication?.id ?? '';
		composeService = task?.compose_service ?? '';
		shell = task?.shell ?? 'bash';
		script = task?.script ?? 'echo "Hello World"';
		timeout = task?.timeout_seconds ?? 3600;
		networkAccess = task?.network_access ?? false;
		serverId = task?.server_id ?? 'local';
		void loadContext().then(() => {
			if (!task && open) serverId = defaultServerId();
		});
	}

	$effect(() => {
		if (open) untrack(reset);
	});

	// Applications: the locked one, or every app in the org, fetched lazily the first
	// time application_exec is picked (no flat endpoint, so it walks each project).
	type ApplicationOption = LockedApplication & { group: string };
	let applicationOptions = $state<ApplicationOption[]>([]);
	let applicationOptionsLoading = $state(false);
	let applicationOptionsLoaded = false;

	async function loadApplicationOptions() {
		if (applicationOptionsLoaded || applicationOptionsLoading) return;
		applicationOptionsLoading = true;
		try {
			const projects = await listProjects(organizationId);
			const details = await Promise.all(
				projects.map((project) => getProject(organizationId, project.id)),
			);
			applicationOptions = details.flatMap(({ project, environments }) =>
				environments.flatMap((environment) =>
					environment.applications.map((application) => ({
						id: application.id,
						label: application.name,
						group: `${project.name} / ${environment.name}`,
						projectId: project.id,
						environmentId: environment.id,
					})),
				),
			);
			applicationOptionsLoaded = true;
		} catch (error) {
			toast.error(error instanceof Error ? error.message : t('organizations.tasks.load_failed'));
		} finally {
			applicationOptionsLoading = false;
		}
	}

	$effect(() => {
		if (open && runner === 'application_exec' && !lockedApplication) void loadApplicationOptions();
	});

	const applicationGroups = $derived(Map.groupBy(applicationOptions, (option) => option.group));
	const selectedApplication = $derived(
		lockedApplication ?? applicationOptions.find((option) => option.id === applicationId) ?? null,
	);

	// services of the picked application; only a multi-service Compose app needs one chosen
	let composeServices = $state<ComposeServiceStatus[]>([]);
	$effect(() => {
		const option = selectedApplication;
		composeServices = [];
		if (!open || runner !== 'application_exec' || !option) return;
		composeStatus(organizationId, option.projectId, option.environmentId, option.id)
			.then((services) => {
				if (option.id === selectedApplication?.id) composeServices = services;
			})
			.catch(() => {});
	});

	const cronText = $derived(describeCron(cron));
	let now = $state(Date.now());
	$effect(() => {
		if (!open) return;
		const timer = setInterval(() => (now = Date.now()), 30_000);
		return () => clearInterval(timer);
	});
	const upcoming = $derived(nextRuns(cron, 3, new Date(now)));
	const utc = (date: Date) => `${date.toISOString().slice(0, 16).replace('T', ' ')} UTC`;

	const target = $derived(
		runner === 'ephemeral'
			? image || t('organizations.tasks.docker_image')
			: runner === 'application_exec'
				? [selectedApplication?.label, composeService].filter(Boolean).join(' / ') ||
					t('organizations.task_ui.runner_application_exec')
				: runner === 'remote_shell'
					? (selectedServer?.name ?? serverId)
					: t('organizations.task_ui.runner_litepod_api'),
	);
	const RunnerIcon = $derived(RUNNER_ICON[runner]);
	const example = $derived(EXAMPLES[runner]);
	const canRun = $derived(!!script && !!serverId && (runner !== 'ephemeral' || !!image));

	function insertExample() {
		script = example.script[shell] ?? example.script.sh ?? script;
		if (example.image) image = example.image;
	}

	function formInput() {
		return {
			name,
			description,
			cron_expression: cron,
			runner,
			image: runner === 'ephemeral' ? image : undefined,
			application_id: runner === 'application_exec' ? applicationId : undefined,
			compose_service:
				runner === 'application_exec' && composeService.trim() ? composeService.trim() : undefined,
			shell,
			script,
			timeout_seconds: timeout,
			network_access: networkAccess,
			server_id: serverId,
		};
	}

	async function save() {
		saving = true;
		try {
			const input = formInput();
			const task = editing
				? await updateScheduledTask(organizationId, editing.id, {
						...input,
						enabled: editing.enabled,
					})
				: await createScheduledTask(organizationId, input);
			toast.success(editing ? t('organizations.tasks.updated') : t('organizations.tasks.created'));
			onSaved?.(task, !editing);
			open = false;
		} catch (error) {
			toast.error(error instanceof Error ? error.message : t('organizations.tasks.save_failed'));
		} finally {
			saving = false;
		}
	}

	// Test run: the current form once, unsaved, streamed inline.
	type TestRun = {
		output: string;
		target: string | null;
		done: Extract<ScheduledTaskTestEvent, { type: 'done' }> | null;
	};
	// the stream keeps at most this much raw output so a chatty script cannot freeze the tab
	const TEST_OUTPUT_LIMIT = 200_000;
	let testing = $state(false);
	let testRun = $state<TestRun | null>(null);
	let testController: AbortController | null = null;
	let testOutputElement = $state<HTMLPreElement | null>(null);
	let testPanelElement = $state<HTMLDivElement | null>(null);
	const testOutput = $derived(testRun ? renderTerminalText(testRun.output) : '');

	$effect(() => {
		void testOutput;
		if (testOutputElement) testOutputElement.scrollTop = testOutputElement.scrollHeight;
	});

	async function testTask() {
		stopTest();
		const controller = new AbortController();
		testController = controller;
		testing = true;
		testRun = { output: '', target: null, done: null };
		const run = testRun;
		// the panel renders below the form, out of view in the scrolling dialog body
		void tick().then(() => testPanelElement?.scrollIntoView({ behavior: 'smooth', block: 'nearest' }));
		try {
			await streamScheduledTaskTest(
				organizationId,
				formInput(),
				(event) => {
					if (event.type === 'started') run.target = event.target;
					else if (event.type === 'output')
						run.output = (run.output + event.data).slice(-TEST_OUTPUT_LIMIT);
					else run.done = event;
				},
				controller.signal,
			);
		} catch (error) {
			if (!controller.signal.aborted) {
				testRun = null;
				toast.error(error instanceof Error ? error.message : t('organizations.tasks.test_failed'));
			}
		} finally {
			if (testController === controller) {
				testController = null;
				testing = false;
			}
		}
	}

	// aborting the request ends the stream, which stops the command on the server
	function stopTest() {
		testController?.abort();
		testController = null;
		testing = false;
	}
</script>

{#snippet sectionHead(title: string, meta: string)}
	<div class="flex items-baseline justify-between gap-3">
		<h3 class="text-[13.5px] font-semibold">{title}</h3>
		<small class="truncate font-mono text-xs text-muted-foreground">{meta}</small>
	</div>
{/snippet}

<Dialog.Root
	bind:open
	onOpenChange={(next) => {
		if (!next) stopTest();
	}}>
	<Dialog.Content
		showCloseButton={false}
		class="flex max-h-[min(88dvh,880px)] flex-col gap-0 overflow-hidden p-0 sm:max-w-2xl">
		<header
			class="lp-groove-b grid grid-cols-[minmax(0,1fr)_auto] items-center gap-x-3 gap-y-0.5 py-3 ps-5 pe-3.5">
			<p class="font-mono text-[10.5px] tracking-[.06em] text-muted-foreground uppercase">
				{editing ? t('organizations.task_ui.kind_edit') : t('organizations.task_ui.kind_new')}
			</p>
			<Dialog.Close>
				{#snippet child({ props })}
					<Button
						{...props}
						variant="ghost"
						size="icon-sm"
						class="row-span-2 self-start bg-secondary"
						aria-label={t('organizations.common.close')}><XIcon /></Button>
				{/snippet}
			</Dialog.Close>
			<Dialog.Title class="truncate text-base leading-tight font-semibold tracking-tight">
				{name || t('organizations.task_ui.untitled')}
			</Dialog.Title>
			<Dialog.Description
				class="col-span-2 mt-0.5 flex flex-wrap items-center gap-x-2.5 gap-y-1 text-xs text-muted-foreground">
				<span class="font-mono text-foreground">{cron}</span>
				<span>{cronText ?? t('organizations.task_ui.invalid_cron')}</span>
				<span class="inline-flex min-w-0 items-center gap-1"
					><RunnerIcon class="size-3.5 shrink-0" /><span class="truncate">{target}</span></span>
				<span class="font-mono">{shell}</span>
			</Dialog.Description>
		</header>

		<div class="grid min-h-0 flex-1 gap-3 overflow-y-auto bg-background/40 p-4">
			<section class="lp-raised grid gap-3 rounded-2xl border bg-card p-4">
				<div class="grid gap-1.5">
					<Label for="task-name">{t('organizations.tasks.task_name')}</Label>
					<Input
						id="task-name"
						bind:value={name}
						autocomplete="off"
						placeholder="nightly-backup"
						autofocus />
				</div>
				<div class="grid gap-1.5">
					<Label for="task-description">{t('organizations.tasks.description_label')}</Label>
					<Input
						id="task-description"
						bind:value={description}
						autocomplete="off"
						placeholder={t('organizations.task_ui.description_placeholder')} />
				</div>
			</section>

			<section class="lp-raised grid gap-3 rounded-2xl border bg-card p-4">
				{@render sectionHead(t('organizations.task_ui.where_runs'), `runner: ${runner}`)}
				<RunnerCards bind:value={runner} {runners} locked={!!lockedApplication} />

				<div class="grid gap-1.5">
					<Label for="task-server">{t('organizations.common.server')}</Label>
					<NativeSelect.Root id="task-server" class="w-full" bind:value={serverId}>
						{#if groups.local.length > 0}
							<NativeSelect.OptGroup label={t('organizations.tasks.group_local')}>
								{#each groups.local as server (server.id)}
									<NativeSelect.Option value={server.id}
										>{server.name} ({server.id})</NativeSelect.Option>
								{/each}
							</NativeSelect.OptGroup>
						{/if}
						{#if groups.remote.length > 0}
							<NativeSelect.OptGroup label={t('organizations.tasks.group_remote')}>
								{#each groups.remote as server (server.id)}
									<NativeSelect.Option value={server.id}
										>{server.name} ({server.id}), {server.state}</NativeSelect.Option>
								{/each}
							</NativeSelect.OptGroup>
						{/if}
						{#if !servers.some((server) => server.id === serverId)}
							<NativeSelect.Option value={serverId}
								>{serverId || t('organizations.common.select_server')}</NativeSelect.Option>
						{/if}
					</NativeSelect.Root>
					{#if selectedServer?.kind === 'ssh'}<p class="text-xs text-muted-foreground">
							{t('organizations.tasks.runs_as', {
								user: selectedServer.ssh_user ?? '?',
								host: selectedServer.ssh_host ?? '?',
								name: selectedServer.name,
							})}
						</p>{/if}
					{#if isCloud && groups.remote.length === 0}<p class="text-xs text-muted-foreground">
							{t('organizations.tasks.no_ready_remote')}
						</p>{/if}
				</div>

				{#if runner === 'ephemeral'}
					<div class="grid gap-1.5">
						<Label for="task-image">{t('organizations.tasks.docker_image')}</Label>
						<Input
							id="task-image"
							class="font-mono"
							bind:value={image}
							placeholder="alpine:3.20"
							autocomplete="off"
							required />
						<p class="text-xs text-muted-foreground">
							{t('organizations.tasks.image_cached_hint')}
						</p>
					</div>
				{:else if runner === 'application_exec'}
					<p class="text-xs text-muted-foreground">
						{t('organizations.tasks.application_exec_hint')}
					</p>
					<div class="grid gap-1.5">
						<Label for="task-app">{t('organizations.tasks.application_label')}</Label>
						{#if lockedApplication}
							<Input id="task-app" value={lockedApplication.label} disabled />
						{:else}
							<NativeSelect.Root id="task-app" class="w-full" bind:value={applicationId}>
								<NativeSelect.Option value="" disabled
									>{applicationOptionsLoading
										? t('organizations.tasks.application_loading')
										: applicationOptions.length === 0
											? t('organizations.tasks.application_none')
											: t('organizations.tasks.application_placeholder')}</NativeSelect.Option>
								{#each applicationGroups as [group, options] (group)}
									<NativeSelect.OptGroup label={group}>
										{#each options as option (option.id)}
											<NativeSelect.Option value={option.id}>{option.label}</NativeSelect.Option>
										{/each}
									</NativeSelect.OptGroup>
								{/each}
							</NativeSelect.Root>
						{/if}
					</div>
					<div class="grid gap-1.5">
						<Label for="task-service">{t('organizations.tasks.compose_service_label')}</Label>
						{#if composeServices.length > 1}
							<div
								class="flex flex-wrap gap-1.5"
								role="radiogroup"
								aria-label={t('organizations.tasks.compose_service_label')}>
								{#each composeServices as service (service.service)}
									<button
										type="button"
										role="radio"
										aria-checked={service.service === composeService}
										title={service.status}
										class={cn(
											'lp-raised inline-flex h-7 items-center gap-1.5 rounded-full border bg-card px-2.5 font-mono text-[11.5px] text-muted-foreground hover:text-foreground',
											service.service === composeService &&
												'border-primary bg-primary/8 text-foreground',
										)}
										onclick={() => (composeService = service.service)}>
										<span
											class={cn(
												'size-1.5 rounded-full',
												isRunning(service.status) ? 'bg-success' : 'bg-muted-foreground/40',
											)}></span>
										{service.service}
									</button>
								{/each}
							</div>
						{:else}
							<Input
								id="task-service"
								class="font-mono"
								bind:value={composeService}
								placeholder="web"
								autocomplete="off" />
						{/if}
						<p class="text-xs text-muted-foreground">
							{t('organizations.tasks.compose_service_hint')}
						</p>
					</div>
				{:else if runner === 'remote_shell'}
					<p class="text-xs text-muted-foreground">{t('organizations.tasks.remote_shell_hint')}</p>
					{#if serverId === 'local'}<p class="text-xs text-destructive">
							{t('organizations.tasks.remote_shell_requires_remote')}
						</p>{/if}
				{/if}
				{#if isCloud}<p class="text-xs text-muted-foreground">
						{t('organizations.tasks.litepod_api_unavailable')}
					</p>{/if}
			</section>

			<section class="lp-raised grid gap-3 rounded-2xl border bg-card p-4">
				{@render sectionHead(t('organizations.task_ui.script'), `shell: ${shell}`)}
				<ShellPicker bind:value={shell} />
				{#if runner === 'ephemeral' && shell !== 'sh' && shell !== 'bash' && image.trim() === DEFAULT_EPHEMERAL_IMAGE}
					<p class="text-xs text-warning">
						{t('organizations.task_ui.shell_image_hint', { shell })}
					</p>
				{/if}
				<ScriptEditor bind:value={script} {shell} height="12rem" filename="task.{shell}">
					{#snippet actions()}
						<button
							type="button"
							class="lp-raised h-6 rounded-full border bg-card px-2.5 font-sans text-[11px] text-muted-foreground hover:text-foreground"
							onclick={insertExample}
							>{t('organizations.task_ui.insert_example', {
								label: t(`organizations.task_ui.${example.label}`),
							})}</button>
					{/snippet}
				</ScriptEditor>
			</section>

			<section class="lp-raised grid gap-3 rounded-2xl border bg-card p-4">
				{@render sectionHead(t('organizations.tasks.schedule'), 'UTC')}
				<div class="grid gap-1.5">
					<Input
						class="font-mono"
						bind:value={cron}
						placeholder="0 0 * * *"
						aria-label={t('organizations.tasks.cron_expression')}
						aria-invalid={!cronText} />
					<p class={cn('text-xs', cronText ? 'text-muted-foreground' : 'text-destructive')}>
						{cronText ?? t('organizations.task_ui.cron_help')}
					</p>
				</div>
				<div class="flex flex-wrap gap-1.5">
					{#each presets as preset (preset.value)}
						<button
							type="button"
							aria-pressed={preset.value === cron}
							class={cn(
								'lp-raised h-6.5 rounded-full border bg-card px-2.5 text-[11.5px] text-muted-foreground hover:text-foreground',
								preset.value === cron && 'border-primary bg-primary/8 text-foreground',
							)}
							onclick={() => (cron = preset.value)}>{preset.label}</button>
					{/each}
				</div>
				{#if upcoming.length}
					<ol
						class="grid gap-1 font-mono text-xs text-muted-foreground"
						aria-label={t('organizations.task_ui.next_runs')}>
						{#each upcoming as run (run.getTime())}
							<li class="lp-well flex justify-between gap-3 rounded-lg bg-muted px-2.5 py-1.5">
								{utc(run)}<span class="text-muted-foreground/70"
									>{relativeTime(run.getTime(), now, i18n.locale)}</span>
							</li>
						{/each}
					</ol>
				{/if}
			</section>

			<section class="lp-raised grid gap-3 rounded-2xl border bg-card p-4">
				{@render sectionHead(t('organizations.task_ui.limits'), `timeout_seconds: ${timeout}`)}
				<div class="grid items-start gap-4 sm:grid-cols-2">
					<div class="grid gap-1.5">
						<Label for="task-timeout">{t('organizations.task_ui.timeout')}</Label>
						<div class="flex items-center gap-2">
							<div
								class="lp-well flex rounded-[10px] bg-muted p-0.5"
								role="group"
								aria-label={t('organizations.task_ui.timeout')}>
								{#each TIMEOUTS as [seconds, label] (seconds)}
									<button
										type="button"
										aria-pressed={timeout === seconds}
										class={cn(
											'rounded-md px-2 py-1 text-xs text-muted-foreground hover:text-foreground',
											timeout === seconds && 'lp-raised bg-card text-foreground',
										)}
										onclick={() => (timeout = seconds)}>{label}</button>
								{/each}
							</div>
							<Input
								id="task-timeout"
								type="number"
								class="w-24 font-mono"
								bind:value={timeout}
								min="1"
								max="86400" />
						</div>
						<p class="text-xs text-muted-foreground">{t('organizations.task_ui.timeout_help')}</p>
					</div>
					{#if runner === 'ephemeral'}
						<div class="flex items-center justify-between gap-3">
							<span>
								<Label for="task-network">{t('organizations.tasks.allow_network')}</Label>
								<small class="mt-1 block text-xs text-muted-foreground"
									>{t('organizations.tasks.network_hint')}</small>
							</span>
							<SoftSwitch id="task-network" bind:checked={networkAccess} />
						</div>
					{/if}
				</div>
			</section>

			{#if testRun}
				{@const done = testRun.done}
				<div
					bind:this={testPanelElement}
					class="overflow-hidden rounded-xl border bg-[#0f1217] text-[#c6ccd7] dark:bg-[#07080a]"
					aria-live="polite">
					<div
						class="flex items-center justify-between gap-2 border-b border-black/50 px-3 py-2 font-mono text-xs text-[#8a93a3]">
						<span class="flex items-center gap-2">
							test-run.log
							{#if done}
								<span class={done.success ? 'text-[#43c983]' : 'text-[#ff6d64]'}
									>{done.success
										? t('organizations.tasks.test_succeeded')
										: t('organizations.tasks.test_failed_result')}{done.exit_code !== null
										? `, ${t('organizations.tasks.exit_code').toLowerCase()} ${done.exit_code}`
										: ''}, {(done.duration_ms / 1000).toFixed(1)}s</span>
							{:else if testing}
								<span class="text-[#f0a83a]">{t('organizations.tasks.testing')}</span>
							{:else}
								<span class="text-[#f0a83a]">{t('organizations.tasks.test_stopped')}</span>
							{/if}
						</span>
						<button
							type="button"
							class="grid size-6 place-items-center rounded-md hover:bg-white/10 hover:text-white"
							aria-label={t('organizations.task_ui.hide_test')}
							onclick={() => {
								stopTest();
								testRun = null;
							}}><XIcon class="size-3.5" /></button>
					</div>
					{#if testRun.target}<p class="px-3 pt-2 font-mono text-xs text-[#8a93a3]">
							{t('organizations.tasks.test_target', { target: testRun.target })}
						</p>{/if}
					{#if done?.error}<p class="px-3 pt-2 font-mono text-xs break-words text-[#ff8f88]">
							{done.error}
						</p>{/if}
					{#if testOutput}
						<pre
							bind:this={testOutputElement}
							class="max-h-60 overflow-auto p-3 font-mono text-xs leading-relaxed break-all whitespace-pre-wrap">{testOutput}</pre>
					{:else if done?.success}
						<p class="p-3 font-mono text-xs text-[#8a93a3]">
							{t('organizations.tasks.test_no_output')}
						</p>
					{:else}
						<p class="p-3">
							<span
								class="inline-block h-3.5 w-2 animate-pulse bg-[#c6ccd7] motion-reduce:animate-none"
							></span>
						</p>
					{/if}
				</div>
			{/if}
		</div>

		<footer class="lp-groove-t flex flex-wrap items-center justify-between gap-2 bg-card px-5 py-3">
			<Button variant="ghost" onclick={() => (open = false)}
				>{t('organizations.common.cancel')}</Button>
			<div class="flex flex-wrap gap-2">
				<Button
					variant="outline"
					disabled={!testing && !canRun}
					onclick={() => (testing ? stopTest() : testTask())}
					>{#if testing}<SquareIcon />{t(
							'organizations.tasks.test_stop',
						)}{:else}<FlaskConicalIcon />{t('organizations.task_ui.test_run')}{/if}</Button>
				<Button disabled={saving || !name || !canRun} onclick={save}>
					{#if editing}<CheckIcon />{:else}<CalendarCheckIcon />{/if}
					{saving
						? t('organizations.common.saving')
						: editing
							? t('organizations.tasks.save_changes')
							: t('organizations.task_ui.create_task')}
				</Button>
			</div>
		</footer>
	</Dialog.Content>
</Dialog.Root>
