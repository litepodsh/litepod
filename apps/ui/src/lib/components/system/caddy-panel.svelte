<script lang="ts">
	import * as Empty from '#lib/components/ui/empty/index.js';
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import RouteIcon from '@lucide/svelte/icons/route';
	import { onMount } from 'svelte';
	import { page } from '$app/state';
	import { goto } from '$app/navigation';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Card from '#lib/components/ui/card/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Label } from '#lib/components/ui/label/index.js';
	import SoftSwitch from '#lib/components/custom/soft-switch.svelte';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import * as Select from '#lib/components/ui/select/index.js';
	import * as Table from '#lib/components/ui/table/index.js';
	import * as AlertDialog from '#lib/components/ui/alert-dialog/index.js';
	import DeleteConfirmControl from '#lib/components/custom/delete-confirm-control.svelte';
	import * as Dialog from '#lib/components/ui/dialog/index.js';
	import { AnimatedTabs } from '#lib/components/custom/animated-tabs/index.js';
	import ContainerLogView from '#lib/components/containers/container-log-view.svelte';
	import ReadOnlyCodeEditor from '#lib/components/read-only-code-editor.svelte';
	import { toast } from '#lib/utils/toast.js';
	import { listServers, type Server } from '#lib/clients/server-client.js';
	import { getSystemVersion } from '#lib/clients/system-client.js';
	import { serverGroups, deploymentTargetNotice } from '#lib/utils/server-visibility.js';
	import {
		getRouteDiff,
		getCaddyfile,
		getCaddyStatus,
		getCaddyHealth,
		getEdgeRoutes,
		reconcileRoutes,
		pruneRoutes,
		listManualRoutes,
		createManualRoute,
		updateManualRoute,
		deleteManualRoute,
		type RouteDiff,
		type RouteDiffStatus,
		type ManualRoute,
		type ManualRouteKind,
		type CaddyHealth,
		type EdgeRoutes,
	} from '#lib/clients/caddy-client.js';
	import RefreshCwIcon from '@lucide/svelte/icons/refresh-cw';
	import PlusIcon from '@lucide/svelte/icons/plus';
	import PencilIcon from '@lucide/svelte/icons/pencil';
	import Trash2Icon from '@lucide/svelte/icons/trash-2';
	import { t } from '#lib/i18n/index.svelte.js';

	type Tab = 'routes' | 'caddyfile' | 'live-config' | 'logs';
	const tabs: { value: Tab; label: string }[] = [
		{ value: 'routes', label: t('organizations.caddy.routes') },
		{ value: 'caddyfile', label: t('organizations.caddy.caddyfile') },
		{ value: 'live-config', label: t('organizations.caddy.live_config') },
		{ value: 'logs', label: t('organizations.caddy.logs') },
	];

	const ROUTE_STATUS: Record<
		RouteDiffStatus,
		{ label: string; variant: 'default' | 'secondary' | 'destructive' | 'outline'; hint: string }
	> = {
		ok: {
			label: t('organizations.caddy.status_ok'),
			variant: 'secondary',
			hint: t('organizations.caddy.hint_ok'),
		},
		missing: {
			label: t('organizations.caddy.status_missing'),
			variant: 'destructive',
			hint: t('organizations.caddy.hint_missing'),
		},
		mismatched: {
			label: t('organizations.caddy.status_mismatched'),
			variant: 'destructive',
			hint: t('organizations.caddy.hint_mismatched'),
		},
		unroutable: {
			label: t('organizations.caddy.status_unroutable'),
			variant: 'outline',
			hint: t('organizations.caddy.hint_unroutable'),
		},
		orphaned: {
			label: t('organizations.caddy.status_orphaned'),
			variant: 'outline',
			hint: t('organizations.caddy.hint_orphaned'),
		},
	};

	let { organizationId }: { organizationId: string } = $props();

	let servers = $state<Server[]>([]);
	let isCloud = $state(false);
	let serverId = $state('');
	let active = $state<Tab>('routes');

	let loading = $state(false);
	let actionBusy = $state(false);
	let error = $state('');

	let routes = $state<RouteDiff | null>(null);
	let manual = $state<ManualRoute[]>([]);
	let caddyfile = $state('');
	let liveConfig = $state('');
	let health = $state<CaddyHealth | null>(null);
	let edgeRoutes = $state<EdgeRoutes>({ rows: [], manages_routes: false, stale_helper: false });

	let serverOptions = $derived.by(() => {
		const groups = serverGroups(servers, isCloud);
		return [...groups.local, ...groups.remote].filter((server) => server.state === 'ready');
	});
	let readyRemoteCount = $derived(
		serverGroups(servers, isCloud).remote.filter((s) => s.state === 'ready').length,
	);
	let targetNotice = $derived(deploymentTargetNotice(isCloud, readyRemoteCount));
	let activeServer = $derived(serverOptions.find((server) => server.id === serverId) ?? null);
	let isCentral = $derived(activeServer?.kind === 'local');
	let orphanCount = $derived(
		isCentral
			? routes
				? routes.entries.filter((e) => e.status === 'orphaned').length
				: 0
			: edgeRoutes.rows.filter((r) => r.status === 'orphaned').length,
	);

	// ---- manual route form --------------------------------------------------
	let showForm = $state(false);
	let editing = $state<ManualRoute | null>(null);
	let form = $state<{
		host: string;
		kind: ManualRouteKind;
		target: string;
		https: boolean;
		redirect_status: number;
	}>({ host: '', kind: 'reverse_proxy', target: '', https: true, redirect_status: 308 });

	function openCreate() {
		editing = null;
		form = { host: '', kind: 'reverse_proxy', target: '', https: true, redirect_status: 308 };
		showForm = true;
	}
	function openEdit(route: ManualRoute) {
		editing = route;
		form = {
			host: route.host,
			kind: route.kind,
			target: route.target,
			https: route.https,
			redirect_status: route.redirect_status,
		};
		showForm = true;
	}

	async function submitForm(event: SubmitEvent) {
		event.preventDefault();
		actionBusy = true;
		try {
			const input = {
				host: form.host.trim(),
				kind: form.kind,
				target: form.target.trim(),
				https: form.https,
				redirect_status: form.redirect_status,
			};
			if (editing) await updateManualRoute(organizationId, serverId, editing.id, input);
			else await createManualRoute(organizationId, serverId, input);
			toast.success(
				editing ? t('organizations.caddy.route_updated') : t('organizations.caddy.route_created'),
			);
			showForm = false;
			await loadAll();
		} catch (err) {
			toast.error(err instanceof Error ? err.message : t('organizations.caddy.save_failed'));
		} finally {
			actionBusy = false;
		}
	}

	// ---- delete -----------------------------------------------------------------
	let pending = $state<{ label: string; expected: string; run: () => Promise<unknown> } | null>(
		null,
	);

	function askDelete(route: ManualRoute) {
		pending = {
			label: t('organizations.caddy.delete_confirm_label', { host: route.host }),
			expected: route.host,
			run: () => deleteManualRoute(organizationId, serverId, route.id, route.host),
		};
	}
	async function confirmPending() {
		if (!pending) return;
		actionBusy = true;
		try {
			await pending.run();
			toast.success(t('organizations.caddy.route_deleted'));
			pending = null;
			await loadAll();
		} catch (err) {
			toast.error(err instanceof Error ? err.message : t('organizations.caddy.delete_failed'));
		} finally {
			actionBusy = false;
		}
	}

	// ---- reconcile / prune ---------------------------------------------------
	async function reconcile() {
		actionBusy = true;
		try {
			const result = await reconcileRoutes(organizationId, serverId);
			if (result.failed > 0)
				toast.error(
					t('organizations.caddy.reconcile_partial', {
						pushed: result.pushed,
						failed: result.failed,
					}),
				);
			else toast.success(t('organizations.caddy.reconcile_pushed', { count: result.pushed }));
			await loadAll();
		} catch (err) {
			toast.error(err instanceof Error ? err.message : t('organizations.caddy.reconcile_failed'));
		} finally {
			actionBusy = false;
		}
	}
	async function prune() {
		actionBusy = true;
		try {
			const { dropped } = await pruneRoutes(organizationId, serverId);
			toast.success(t('organizations.caddy.pruned_orphans', { count: dropped }));
			await loadAll();
		} catch (err) {
			toast.error(err instanceof Error ? err.message : t('organizations.caddy.prune_failed'));
		} finally {
			actionBusy = false;
		}
	}

	// ---- loading -----------------------------------------------------------
	async function loadAll() {
		if (!serverId) return;
		loading = true;
		error = '';
		try {
			if (isCentral) {
				const [diff, cfg, status, list] = await Promise.allSettled([
					getRouteDiff(organizationId, serverId),
					getCaddyfile(organizationId, serverId),
					getCaddyStatus(organizationId, serverId),
					listManualRoutes(organizationId, serverId),
				]);
				routes = diff.status === 'fulfilled' ? diff.value : null;
				caddyfile = cfg.status === 'fulfilled' ? cfg.value.content : '';
				liveConfig =
					status.status === 'fulfilled' ? JSON.stringify(status.value.active_config, null, 2) : '';
				manual = list.status === 'fulfilled' ? list.value : [];
				if (diff.status === 'rejected')
					error =
						diff.reason instanceof Error
							? diff.reason.message
							: t('organizations.caddy.api_failed');
			} else {
				const [h, list] = await Promise.allSettled([
					getCaddyHealth(organizationId, serverId),
					getEdgeRoutes(organizationId, serverId),
				]);
				health = h.status === 'fulfilled' ? h.value : null;
				edgeRoutes =
					list.status === 'fulfilled'
						? list.value
						: { rows: [], manages_routes: false, stale_helper: false };
				if (h.status === 'rejected')
					error =
						h.reason instanceof Error ? h.reason.message : t('organizations.caddy.edge_failed');
				routes = null;
				manual = [];
			}
		} catch (err) {
			error = err instanceof Error ? err.message : t('organizations.caddy.load_failed');
		} finally {
			loading = false;
		}
	}

	function setServerQuery(id: string) {
		const url = new URL(page.url.href);
		url.searchParams.set('server', id);
		void goto(`${url.pathname}${url.search}`, {
			replace: true,
			reset: false,
		});
	}
	async function changeServer(id: string) {
		serverId = id;
		setServerQuery(id);
		if (!isCentral && active !== 'routes') active = 'routes';
		await loadAll();
	}

	onMount(async () => {
		try {
			const [serverList, version] = await Promise.all([
				listServers(organizationId),
				getSystemVersion().catch(() => ({ is_cloud: false }) as { is_cloud: boolean }),
			]);
			servers = serverList;
			isCloud = version.is_cloud;
			const fromQuery = page.url.searchParams.get('server');
			serverId =
				(fromQuery && serverOptions.some((s) => s.id === fromQuery) && fromQuery) ||
				serverOptions[0]?.id ||
				'';
			if (serverId) await loadAll();
		} catch (err) {
			error = err instanceof Error ? err.message : t('organizations.common.servers_load_failed');
		}
	});
</script>

<div class="flex flex-col gap-4">
	<div class="flex flex-wrap items-end justify-between gap-3">
		<div class="min-w-0">
			<h2 class="text-lg font-semibold tracking-tight">{t('organizations.caddy.title')}</h2>
			<p class="mt-1 text-sm text-muted-foreground">
				{t('organizations.caddy.description')}
			</p>
		</div>
		<div class="flex w-full flex-wrap items-center gap-2 sm:w-auto">
			<Select.Root type="single" value={serverId} onValueChange={changeServer}>
				<Select.Trigger class="w-full sm:w-56" aria-label={t('organizations.common.server')}>
					{activeServer ? activeServer.name : t('organizations.common.select_server')}
				</Select.Trigger>
				<Select.Content>
					{#each serverOptions as server (server.id)}
						<Select.Item value={server.id} label={server.name}>
							{server.name}
							<span class="ml-2 text-xs text-muted-foreground"
								>{server.kind === 'local'
									? t('organizations.caddy.central')
									: t('organizations.common.remote')}</span>
						</Select.Item>
					{/each}
				</Select.Content>
			</Select.Root>
			<Button variant="outline" onclick={loadAll} disabled={loading || actionBusy || !serverId}>
				<RefreshCwIcon class={loading ? 'animate-spin' : ''} />{t('organizations.common.refresh')}
			</Button>
		</div>
	</div>

	{#if serverOptions.length === 0}
		<div
			class="rounded-2xl border border-dashed border-foreground/15 bg-card p-8 text-center text-sm text-muted-foreground">
			{targetNotice ?? t('organizations.common.no_ready_servers')}
		</div>
	{:else if !isCentral}
		<Card.Root>
			<Card.Header>
				<Card.Title>{t('organizations.caddy.edge_proxy')}</Card.Title>
				<Card.Description>
					{t('organizations.caddy.edge_description', { name: activeServer?.name ?? '' })}
				</Card.Description>
			</Card.Header>
			<Card.Content>
				{#if loading}
					<Skeleton class="h-16 rounded-xl" />
				{:else if health}
					<div class="flex items-center gap-2">
						<Badge variant={health.healthy ? 'secondary' : 'destructive'}>
							{health.healthy
								? t('organizations.caddy.healthy')
								: t('organizations.caddy.unreachable')}
						</Badge>
						<span class="text-sm text-muted-foreground">{health.detail}</span>
					</div>
				{/if}
			</Card.Content>
		</Card.Root>

		{#if error}
			<div class="rounded-2xl border border-destructive/30 bg-card p-4 text-sm text-destructive">
				{error}
			</div>
		{/if}

		<Card.Root>
			<Card.Header>
				<Card.Title>{t('organizations.caddy.application_routes')}</Card.Title>
				<Card.Description>
					{#if edgeRoutes.manages_routes}
						{t('organizations.caddy.app_routes_managed')}
					{:else}
						{t('organizations.caddy.app_routes_central')}
					{/if}
				</Card.Description>
			</Card.Header>
			<Card.Content class="space-y-4">
				{#if edgeRoutes.manages_routes && edgeRoutes.stale_helper}
					<div class="rounded-md border border-warning bg-warning/10 p-3 text-sm text-warning">
						{t('organizations.caddy.stale_helper_warning')}
					</div>
				{/if}

				{#if edgeRoutes.manages_routes}
					<div class="flex gap-2">
						<Button variant="outline" size="sm" onclick={reconcile} disabled={actionBusy}>
							<RefreshCwIcon class="mr-1.5 size-3.5" />
							{t('organizations.caddy.reconcile_routes')}
						</Button>
						<Button
							variant="outline"
							size="sm"
							onclick={prune}
							disabled={actionBusy || edgeRoutes.stale_helper || orphanCount === 0}>
							{t('organizations.caddy.prune_orphans', { count: orphanCount })}
						</Button>
					</div>
				{/if}

				{#if loading}
					<Skeleton class="h-24 rounded-xl" />
				{:else if edgeRoutes.rows.length === 0}
					<Empty.Root class="rounded-2xl border border-dashed border-foreground/15">
						<Empty.Header>
							<Empty.Media variant="icon"><RouteIcon /></Empty.Media>
							<Empty.Title>{t('organizations.caddy.empty')}</Empty.Title>
							<Empty.Description>
								{t('organizations.caddy.empty_description')}
							</Empty.Description>
						</Empty.Header>
					</Empty.Root>
				{:else}
					<div class="lp-inset overflow-x-auto rounded-xl border border-foreground/9">
						<Table.Root>
							<Table.Header>
								<Table.Row>
									<Table.Head>{t('organizations.caddy.host')}</Table.Head>
									<Table.Head>{t('organizations.caddy.application')}</Table.Head>
									<Table.Head>{t('organizations.caddy.status')}</Table.Head>
								</Table.Row>
							</Table.Header>
							<Table.Body>
								{#each edgeRoutes.rows as route (route.domain_id)}
									<Table.Row>
										<Table.Cell class="font-mono text-xs"
											>{route.host || route.domain_id}</Table.Cell>
										<Table.Cell class="text-sm">{route.application_name || '—'}</Table.Cell>
										<Table.Cell>
											<Badge
												variant={route.status === 'ok'
													? 'secondary'
													: route.status === 'orphaned' || route.status === 'missing'
														? 'destructive'
														: 'outline'}>
												{route.status}
											</Badge>
										</Table.Cell>
									</Table.Row>
								{/each}
							</Table.Body>
						</Table.Root>
					</div>
				{/if}
			</Card.Content>
		</Card.Root>
	{:else}
		{#if error}
			<div class="rounded-2xl border border-destructive/30 bg-card p-4 text-sm text-destructive">
				{error}
			</div>
		{/if}

		<Card.Root>
			<Card.Header class="gap-4">
				<AnimatedTabs bind:value={active} items={tabs} />
			</Card.Header>
			<Card.Content class="flex flex-col gap-6">
				{#if active === 'routes'}
					<!-- Manual routes -->
					<section class="flex flex-col gap-3">
						<div class="flex flex-wrap items-center justify-between gap-2">
							<div>
								<h3 class="text-[15px] font-semibold">{t('organizations.caddy.manual_routes')}</h3>
								<p class="text-sm text-muted-foreground">
									{t('organizations.caddy.manual_routes_description')}
								</p>
							</div>
							<Button size="sm" onclick={openCreate} disabled={actionBusy}>
								<PlusIcon />{t('organizations.caddy.add_route')}
							</Button>
						</div>
						{#if manual.length === 0}
							<p class="text-sm text-muted-foreground">
								{t('organizations.caddy.manual_routes_empty')}
							</p>
						{:else}
							<div class="lp-inset overflow-x-auto rounded-xl border border-foreground/9">
								<Table.Root>
									<Table.Header>
										<Table.Row>
											<Table.Head>{t('organizations.caddy.host')}</Table.Head>
											<Table.Head>{t('organizations.caddy.type')}</Table.Head>
											<Table.Head>{t('organizations.caddy.target')}</Table.Head>
											<Table.Head>{t('organizations.caddy.https')}</Table.Head>
											<Table.Head class="text-right">{t('organizations.caddy.actions')}</Table.Head>
										</Table.Row>
									</Table.Header>
									<Table.Body>
										{#each manual as route (route.id)}
											<Table.Row>
												<Table.Cell class="font-mono text-xs">{route.host}</Table.Cell>
												<Table.Cell class="text-sm">
													{route.kind === 'redirect'
														? t('organizations.caddy.redirect_display', {
																status: route.redirect_status,
															})
														: t('organizations.caddy.reverse_proxy')}
												</Table.Cell>
												<Table.Cell class="font-mono text-xs">{route.target}</Table.Cell>
												<Table.Cell class="text-sm"
													>{route.https
														? t('organizations.caddy.on')
														: t('organizations.caddy.off')}</Table.Cell>
												<Table.Cell class="text-right">
													<Button
														variant="ghost"
														size="icon"
														aria-label={t('organizations.common.edit')}
														onclick={() => openEdit(route)}
														disabled={actionBusy}><PencilIcon /></Button>
													<Button
														variant="ghost"
														size="icon"
														aria-label={t('organizations.common.delete')}
														onclick={() => askDelete(route)}
														disabled={actionBusy}><Trash2Icon /></Button>
												</Table.Cell>
											</Table.Row>
										{/each}
									</Table.Body>
								</Table.Root>
							</div>
						{/if}
					</section>

					<!-- Application routes -->
					<section class="flex flex-col gap-3">
						<div class="flex flex-wrap items-center justify-between gap-2">
							<div>
								<h3 class="text-[15px] font-semibold">
									{t('organizations.caddy.application_routes')}
								</h3>
								<p class="text-sm text-muted-foreground">
									{t('organizations.caddy.app_routes_description')}
								</p>
							</div>
							<div class="flex gap-2">
								{#if orphanCount > 0}
									<Button variant="outline" size="sm" onclick={prune} disabled={actionBusy}>
										{t('organizations.caddy.prune_orphans', { count: orphanCount })}
									</Button>
								{/if}
								<Button
									size="sm"
									onclick={reconcile}
									disabled={actionBusy || !routes || routes.needs_reconcile === 0}>
									{#if routes && routes.needs_reconcile > 0}
										{t('organizations.caddy.reconcile_count', { count: routes.needs_reconcile })}
									{:else}
										{t('organizations.caddy.nothing_to_reconcile')}
									{/if}
								</Button>
							</div>
						</div>
						{#if loading}
							<Skeleton class="h-40 rounded-xl" />
						{:else if !routes || routes.entries.length === 0}
							<p class="text-sm text-muted-foreground">
								{t('organizations.caddy.app_routes_empty')}
							</p>
						{:else}
							<div class="lp-inset overflow-x-auto rounded-xl border border-foreground/9">
								<Table.Root>
									<Table.Header>
										<Table.Row>
											<Table.Head>{t('organizations.caddy.host')}</Table.Head>
											<Table.Head>{t('organizations.caddy.owner')}</Table.Head>
											<Table.Head>{t('organizations.caddy.status')}</Table.Head>
											<Table.Head>{t('organizations.caddy.expected')}</Table.Head>
											<Table.Head>{t('organizations.caddy.live')}</Table.Head>
										</Table.Row>
									</Table.Header>
									<Table.Body>
										{#each routes.entries as entry (entry.route_id)}
											<Table.Row>
												<Table.Cell class="font-mono text-xs">{entry.host}</Table.Cell>
												<Table.Cell class="text-sm">
													{entry.source === 'manual' ? 'manual' : entry.application_name || '—'}
												</Table.Cell>
												<Table.Cell>
													<Badge variant={ROUTE_STATUS[entry.status].variant}>
														{ROUTE_STATUS[entry.status].label}
													</Badge>
												</Table.Cell>
												<Table.Cell class="font-mono text-xs"
													>{entry.expected_dial ?? '—'}</Table.Cell>
												<Table.Cell
													class="font-mono text-xs {entry.status === 'mismatched'
														? 'text-destructive'
														: ''}">{entry.live_dial ?? '—'}</Table.Cell>
											</Table.Row>
										{/each}
									</Table.Body>
								</Table.Root>
							</div>
							<dl class="grid gap-1 text-xs text-muted-foreground sm:grid-cols-2">
								{#each Object.entries(ROUTE_STATUS) as [key, meta] (key)}
									<div class="flex gap-2">
										<dt class="font-medium">{meta.label}:</dt>
										<dd>{meta.hint}</dd>
									</div>
								{/each}
							</dl>
						{/if}
					</section>
				{:else if active === 'caddyfile'}
					{#if loading}
						<Skeleton class="min-h-[16rem] rounded-xl" />
					{:else}
						<ReadOnlyCodeEditor
							content={caddyfile}
							language="caddyfile"
							label={t('organizations.caddy.managed_caddyfile')} />
					{/if}
				{:else if active === 'logs'}
					{#key serverId}
						<ContainerLogView {organizationId} {serverId} containerId="litepod-caddy" />
					{/key}
				{:else if loading}
					<Skeleton class="min-h-[16rem] rounded-xl" />
				{:else}
					<ReadOnlyCodeEditor
						content={liveConfig}
						language="json"
						label={t('organizations.caddy.live_admin_config')} />
				{/if}
			</Card.Content>
		</Card.Root>
	{/if}
</div>

<!-- Create / edit manual route -->
<Dialog.Root bind:open={showForm}>
	<Dialog.Content>
		<Dialog.Header>
			<Dialog.Title
				>{editing
					? t('organizations.caddy.edit_route')
					: t('organizations.caddy.add_route')}</Dialog.Title>
			<Dialog.Description>
				{t('organizations.caddy.form_description')}
			</Dialog.Description>
		</Dialog.Header>
		<form class="flex flex-col gap-4" onsubmit={submitForm}>
			<div class="grid gap-1.5">
				<Label for="mr-host">{t('organizations.caddy.host')}</Label>
				<Input id="mr-host" bind:value={form.host} placeholder="app.example.com" required />
			</div>
			<div class="grid gap-1.5">
				<Label for="mr-kind">{t('organizations.caddy.type')}</Label>
				<Select.Root
					type="single"
					value={form.kind}
					onValueChange={(v) => (form.kind = v as ManualRouteKind)}>
					<Select.Trigger id="mr-kind">
						{form.kind === 'redirect'
							? t('organizations.caddy.redirect')
							: t('organizations.caddy.reverse_proxy')}
					</Select.Trigger>
					<Select.Content>
						<Select.Item value="reverse_proxy" label={t('organizations.caddy.reverse_proxy')}
							>{t('organizations.caddy.reverse_proxy')}</Select.Item>
						<Select.Item value="redirect" label={t('organizations.caddy.redirect')}
							>{t('organizations.caddy.redirect')}</Select.Item>
					</Select.Content>
				</Select.Root>
			</div>
			<div class="grid gap-1.5">
				<Label for="mr-target"
					>{form.kind === 'redirect'
						? t('organizations.caddy.destination')
						: t('organizations.caddy.upstream')}</Label>
				<Input
					id="mr-target"
					bind:value={form.target}
					placeholder={form.kind === 'redirect' ? 'https://example.com' : '127.0.0.1:8080'}
					required />
			</div>
			{#if form.kind === 'redirect'}
				<div class="grid gap-1.5">
					<Label for="mr-status">{t('organizations.caddy.redirect_status')}</Label>
					<Select.Root
						type="single"
						value={String(form.redirect_status)}
						onValueChange={(v) => (form.redirect_status = Number(v))}>
						<Select.Trigger id="mr-status">{form.redirect_status}</Select.Trigger>
						<Select.Content>
							<Select.Item value="308" label="308"
								>308 — {t('organizations.caddy.permanent')}</Select.Item>
							<Select.Item value="301" label="301"
								>301 — {t('organizations.caddy.permanent_legacy')}</Select.Item>
							<Select.Item value="307" label="307"
								>307 — {t('organizations.caddy.temporary')}</Select.Item>
							<Select.Item value="302" label="302"
								>302 — {t('organizations.caddy.temporary_legacy')}</Select.Item>
						</Select.Content>
					</Select.Root>
				</div>
			{/if}
			<label class="flex items-center gap-2 text-sm">
				<SoftSwitch bind:checked={form.https} />
				{t('organizations.caddy.automatic_https')}
			</label>
			<Dialog.Footer>
				<Button type="button" variant="outline" onclick={() => (showForm = false)}
					>{t('organizations.common.cancel')}</Button>
				<Button type="submit" disabled={actionBusy}>
					{editing ? t('organizations.common.save') : t('organizations.common.create')}
				</Button>
			</Dialog.Footer>
		</form>
	</Dialog.Content>
</Dialog.Root>

<!-- Delete confirmation -->
<AlertDialog.Root
	open={pending !== null}
	onOpenChange={(open) => {
		if (!open && !actionBusy) {
			pending = null;
		}
	}}>
	<AlertDialog.Content>
		<AlertDialog.Header>
			<AlertDialog.Title>{t('organizations.caddy.delete_route')}</AlertDialog.Title>
			<AlertDialog.Description>
				{t('organizations.caddy.delete_description_before')}<strong>{pending?.expected}</strong>{t(
					'organizations.caddy.delete_description_after',
				)}
			</AlertDialog.Description>
		</AlertDialog.Header>
		<AlertDialog.Footer>
			<AlertDialog.Cancel disabled={actionBusy}
				>{t('organizations.common.cancel')}</AlertDialog.Cancel>
			<DeleteConfirmControl disabled={!pending || actionBusy} onconfirm={confirmPending} />
		</AlertDialog.Footer>
	</AlertDialog.Content>
</AlertDialog.Root>
