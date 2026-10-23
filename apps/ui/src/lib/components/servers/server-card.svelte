<script lang="ts">
	import ActivityIcon from '@lucide/svelte/icons/activity';
	import BoxIcon from '@lucide/svelte/icons/box';
	import ChartLineIcon from '@lucide/svelte/icons/chart-line';
	import BoxesIcon from '@lucide/svelte/icons/boxes';
	import PlusIcon from '@lucide/svelte/icons/plus';
	import ContainerIcon from '@lucide/svelte/icons/container';
	import EllipsisVerticalIcon from '@lucide/svelte/icons/ellipsis-vertical';
	import KeyRoundIcon from '@lucide/svelte/icons/key-round';
	import LayersIcon from '@lucide/svelte/icons/layers';
	import NetworkIcon from '@lucide/svelte/icons/network';
	import PencilIcon from '@lucide/svelte/icons/pencil';
	import PlayIcon from '@lucide/svelte/icons/play';
	import Trash2Icon from '@lucide/svelte/icons/trash-2';
	import WrenchIcon from '@lucide/svelte/icons/wrench';
	import TerminalIcon from '@lucide/svelte/icons/terminal';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Checkbox } from '#lib/components/ui/checkbox/index.js';
	import * as Card from '#lib/components/ui/card/index.js';
	import * as DropdownMenu from '#lib/components/ui/dropdown-menu/index.js';
	import CellChart from '#lib/components/custom/cell-chart.svelte';
	import CopyButton from '#lib/components/custom/copy-button.svelte';
	import OsIcon from '#lib/components/os-icon.svelte';
	import ServerHelperNotice from './server-helper-notice.svelte';
	import ServerSwapNotice from './server-swap-notice.svelte';
	import { getServerMetricsHistory, totalSeries } from '#lib/clients/container-metrics-client.js';
	import { getSelectedOrganizationId } from '#lib/utils/selected-organization.js';
	import { osLabel } from '#lib/utils/os-label.js';
	import type { RuntimeStatus } from '#lib/clients/runtime-client.js';
	import type { ServerGroup } from '#lib/clients/server-group-client.js';
	import { serverActionLabel, type Server, type ServerState } from '#lib/clients/server-client.js';
	import { i18n, t } from '#lib/i18n/index.svelte.js';

	let {
		server,
		busy = false,
		runtime = null,
		canManage = true,
		onContinue,
		onShowKey,
		onHealthcheck,
		onMaintenance,
		onTestDeploy,
		onRepair,
		onRename,
		onDelete,
		onOpenConsole,
		groups,
		onToggleGroup,
		onNewGroup,
		selected = false,
		onSelectedChange,
	}: {
		server: Server;
		busy?: boolean;
		/** The local server keeps no probed facts, so its mark comes from the running host. */
		runtime?: RuntimeStatus | null;
		/** Members below Admin see the card but get no actions on it. */
		canManage?: boolean;
		onContinue: (server: Server) => void;
		onShowKey: (server: Server) => void;
		onHealthcheck: (server: Server) => void;
		onMaintenance: (server: Server) => void;
		onTestDeploy: (server: Server) => void;
		onRepair: (server: Server) => void;
		onRename: (server: Server) => void;
		onDelete: (server: Server) => void;
		onOpenConsole?: (server: Server) => void;
		/** Link to this server's application/network Map. */
		/** Server groups; passing them adds the Groups submenu to a remote card's actions. */
		groups?: ServerGroup[];
		onToggleGroup?: (server: Server, group: ServerGroup, member: boolean) => void;
		onNewGroup?: (server: Server) => void;
		/** Passing `onSelectedChange` shows a checkbox so several cards can be grouped at once. */
		selected?: boolean;
		onSelectedChange?: (server: Server, selected: boolean) => void;
	} = $props();

	const STATE_LABELS: Record<ServerState, string> = {
		needs_verification: t('organizations.servers.needs_verification'),
		needs_setup: t('organizations.servers.needs_setup'),
		setting_up: t('organizations.servers.setting_up'),
		ready: t('organizations.servers.ready'),
		degraded: t('organizations.servers.degraded'),
		offline: t('organizations.servers.offline'),
		unsupported: t('organizations.servers.unsupported'),
	};

	// Ready is the only healthy state, so everything else reads as a warning or an error.
	const STATE_CLASSES: Record<ServerState, string> = {
		ready: 'border-success/40 bg-success/10 text-success',
		setting_up: 'border-info/40 bg-info/10 text-info',
		needs_verification: 'border-warning/40 bg-warning/10 text-warning',
		needs_setup: 'border-warning/40 bg-warning/10 text-warning',
		degraded: 'border-destructive/40 bg-destructive/10 text-destructive',
		offline: 'border-destructive/40 bg-destructive/10 text-destructive',
		unsupported: 'border-destructive/40 bg-destructive/10 text-destructive',
	};

	const isRemote = $derived(server.kind === 'ssh');
	const needsAttention = $derived(isRemote && server.state !== 'ready');
	const address = $derived(
		isRemote
			? `${server.ssh_user}@${server.ssh_host}:${server.ssh_port}`
			: (server.public_address ?? t('organizations.servers.installed_locally')),
	);

	// Remote facts only exist once a server has been probed, so an unverified host shows no mark.
	const os = $derived(
		isRemote
			? osLabel(server.facts.os, server.facts.version)
			: osLabel(runtime?.host_distribution ?? runtime?.host_os),
	);

	function gib(bytes: number | null): string | null {
		return bytes ? `${(bytes / 1024 ** 3).toFixed(1)} GiB` : null;
	}

	/** Probed facts as label/value cells; a fact the server never reported is left out. */
	const specs = $derived(
		(
			[
				['os', [os, !isRemote ? runtime?.host_version : null].filter(Boolean).join(' ') || null],
				[
					'cpu',
					server.facts.capacity.cpu_cores
						? t('servers.card.vcpu', { count: server.facts.capacity.cpu_cores })
						: null,
				],
				['memory', gib(server.facts.capacity.memory_bytes)],
				[
					'disk',
					gib(server.facts.capacity.disk_free_bytes)
						? t('servers.card.free', { value: gib(server.facts.capacity.disk_free_bytes) })
						: null,
				],
				// Remote cards sit three abreast, so they keep to the four facts that size a host.
				['podman', isRemote ? null : server.facts.podman_version],
				['arch', isRemote ? null : server.facts.architecture],
				['kernel', !isRemote ? (runtime?.host_kernel_version ?? null) : null],
			] as [string, string | null | undefined][]
		).filter((spec): spec is [string, string] => Boolean(spec[1])),
	);

	// 24h of CPU (sum of its containers), RAM and disk on this host, from the metrics store.
	// Laid on a fixed 24h grid so a young host shows empty columns, not stretched data.
	// RAM and disk keep the capacity of each moment, so a resize shows as unavailable cells.
	const DAY = 24 * 3600_000;
	const cols = $derived(isRemote ? 24 : 48);
	type Point = { ts: number; value: number | null; cap?: number | null };
	let history = $state<{
		cpu: Point[];
		mem: Point[];
		disk: Point[];
		hostMemory: boolean;
		end: number;
	} | null>(null);
	$effect(() => {
		if (server.state !== 'ready' && server.state !== 'degraded') return;
		const organizationId = server.organization_id ?? getSelectedOrganizationId();
		if (!organizationId) return;
		getServerMetricsHistory(organizationId, server.id, '24h')
			.then((data) => {
				const series = totalSeries(data);
				const hostMemory = data.host.some((p) => p.mem_used !== null);
				history =
					series.ts.length || data.host.length
						? {
								cpu: series.ts.map((ts, i) => ({ ts, value: series.cpuAvg[i] })),
								mem: hostMemory
									? data.host.map((p) => ({ ts: p.ts, value: p.mem_used, cap: p.mem_total }))
									: series.ts.map((ts, i) => ({ ts, value: series.memAvg[i] })),
								disk: data.host.map((p) => ({ ts: p.ts, value: p.disk_used, cap: p.disk_total })),
								hostMemory,
								end: Date.now(),
							}
						: null;
			})
			.catch(() => (history = null));
	});
	const slotMs = $derived(DAY / cols);
	// Mean value and largest capacity of the buckets in each slot; null where nothing was reported.
	function slotted(points: Point[]): { values: (number | null)[]; caps: (number | null)[] } {
		const sum = Array<number>(cols).fill(0);
		const count = Array<number>(cols).fill(0);
		const caps = Array<number | null>(cols).fill(null);
		if (!history) return { values: [], caps: [] };
		const start = history.end - DAY;
		for (const point of points) {
			const slot = Math.floor((point.ts - start) / slotMs);
			if (slot < 0 || slot >= cols) continue;
			if (point.value !== null) {
				sum[slot] += point.value;
				count[slot]++;
			}
			if (point.cap != null) caps[slot] = Math.max(caps[slot] ?? 0, point.cap);
		}
		return { values: sum.map((total, i) => (count[i] ? total / count[i] : null)), caps };
	}
	const cpuSlots = $derived(history ? slotted(history.cpu).values : []);
	const cpu = $derived(cpuSlots.map((v) => v ?? 0));
	const memoryBytes = $derived(server.facts?.capacity?.memory_bytes ?? null);
	const memSeries = $derived(history ? slotted(history.mem) : { values: [], caps: [] });
	// Without host samples yet, the probed RAM size stands in for every column.
	const memCaps = $derived(
		history?.hostMemory ? memSeries.caps : memSeries.values.map(() => memoryBytes),
	);
	const diskSeries = $derived(history ? slotted(history.disk) : { values: [], caps: [] });
	const hasDisk = $derived(diskSeries.values.some((v) => v !== null));
	const lastIndex = (values: (number | null)[]) =>
		Math.max(
			0,
			values.findLastIndex((v) => v !== null),
		);
	const peakOf = (values: (number | null)[]) =>
		Math.max(0, ...values.filter((v): v is number => v !== null));
	let hovered = $state<number | null>(null);
	function clock(ms: number): string {
		return new Date(ms).toLocaleTimeString(i18n.locale, { hour: '2-digit', minute: '2-digit' });
	}
	const hoverRange = $derived.by(() => {
		if (hovered === null || !history) return '';
		const from = history.end - DAY + hovered * slotMs;
		return `${clock(from)}–${clock(from + slotMs)}`;
	});
	const percent = (v: number) => `${Math.round(v)}%`;
	function bytesLabel(values: (number | null)[], caps: (number | null)[], i: number): string {
		const value = values[i];
		if (value == null) return t('servers.card.no_data');
		const cap = caps[i];
		return cap
			? `${percent((value / cap) * 100)} · ${gib(value) ?? '0.0 GiB'} / ${gib(cap)}`
			: (gib(value) ?? '0.0 GiB');
	}
	function bytesTone(values: (number | null)[], caps: (number | null)[]): 'success' | 'warning' {
		const i = lastIndex(values);
		const cap = caps[i];
		return cap && (values[i] ?? 0) / cap >= 0.85 ? 'warning' : 'success';
	}
	const historyHref = $derived(
		`/organizations/${encodeURIComponent(server.organization_id ?? getSelectedOrganizationId() ?? '')}/servers/${encodeURIComponent(server.id)}`,
	);
</script>

{#snippet usage(
	title: string,
	values: number[],
	slots: (number | null)[],
	current: string,
	at: (i: number) => string,
	max: number,
	tone: 'primary' | 'success' | 'warning',
	format: (v: number) => string,
	caps?: (number | null)[],
)}
	<div class="flex min-w-0 flex-col gap-1">
		<div class="flex items-center justify-between gap-2 text-xs text-muted-foreground">
			{#if hovered !== null}
				<span class="truncate font-mono">{hoverRange}</span>
				<b class="font-mono text-[13px] font-medium whitespace-nowrap text-foreground"
					>{slots[hovered] === null ? t('servers.card.no_data') : at(hovered)}</b>
			{:else}
				<span class="truncate">{title}</span>
				<b class="font-mono text-[13px] font-medium whitespace-nowrap text-foreground">{current}</b>
			{/if}
		</div>
		<CellChart
			{values}
			{cols}
			{max}
			onhover={(i) => (hovered = i)}
			rows={isRemote ? 7 : 10}
			{tone}
			{format}
			{caps}
			ariaLabel={title} />
	</div>
{/snippet}

<Card.Root
	class="h-full gap-4 px-4.5 [--card-spacing:--spacing(4.5)] {server.state === 'offline'
		? 'opacity-80'
		: ''}">
	<div class="flex flex-wrap items-start justify-between gap-2.5">
		<!-- Wide enough for a name; the actions wrap under it on a phone. -->
		<div class="min-w-48 flex-1">
			<div class="flex flex-wrap items-center gap-2">
				{#if onSelectedChange}
					<Checkbox
						checked={selected}
						onCheckedChange={(next) => onSelectedChange(server, next)}
						aria-label={t('organizations.server_groups.select_server', { name: server.name })} />
				{/if}
				{#if os}
					<span class="flex shrink-0 items-center" title={os}>
						<OsIcon
							os={isRemote ? null : runtime?.host_os}
							distribution={isRemote ? server.facts.os : runtime?.host_distribution}
							class="size-4" />
						<span class="sr-only">{os}</span>
					</span>
				{/if}
				<h3 class="truncate text-[15px] font-semibold">{server.name}</h3>
				<Badge variant="outline" class={STATE_CLASSES[server.state]}
					>{STATE_LABELS[server.state]}</Badge>
			</div>
			<p class="mt-0.5 truncate font-mono text-[12.5px] text-muted-foreground">{address}</p>
		</div>

		<div class="flex shrink-0 items-center gap-1.5">
			{#if server.state === 'ready' || server.state === 'degraded'}
				<Button
					size={isRemote ? 'icon-sm' : 'sm'}
					variant="outline"
					href={historyHref}
					title={t('servers.card.history')}
					aria-label={t('servers.card.history')}>
					<ChartLineIcon class="size-4" />
					{#if !isRemote}{t('servers.card.history')}{/if}
				</Button>
			{/if}
			{#if server.state === 'ready' && onOpenConsole}
				<Button
					size={isRemote ? 'icon-sm' : 'sm'}
					variant="outline"
					disabled={busy}
					title={t('organizations.servers.open_console')}
					aria-label={t('organizations.servers.open_console')}
					onclick={() => onOpenConsole(server)}>
					<TerminalIcon class="size-4" />
					{#if !isRemote}{t('organizations.servers.open_console')}{/if}
				</Button>
			{/if}
			{#if isRemote && canManage}
				{#if needsAttention}
					<Button size="sm" disabled={busy} onclick={() => onContinue(server)}>
						<PlayIcon class="size-4" />
						{server.state === 'setting_up'
							? t('servers.card.view_setup')
							: serverActionLabel(server)}
					</Button>
				{/if}
				<DropdownMenu.Root>
					<DropdownMenu.Trigger>
						{#snippet child({ props })}
							<Button
								{...props}
								size="icon-sm"
								variant="outline"
								disabled={busy}
								aria-label={t('organizations.servers.actions')}>
								<EllipsisVerticalIcon class="size-4" />
							</Button>
						{/snippet}
					</DropdownMenu.Trigger>
					<DropdownMenu.Content align="end">
						<DropdownMenu.Item onSelect={() => onHealthcheck(server)}>
							<ActivityIcon class="size-4" />
							{t('organizations.servers.health_check')}
						</DropdownMenu.Item>
						<DropdownMenu.Item onSelect={() => onTestDeploy(server)}>
							<ContainerIcon class="size-4" />
							{t('organizations.servers.test_deploy_action')}
						</DropdownMenu.Item>
						<DropdownMenu.Item onSelect={() => onRepair(server)}>
							<WrenchIcon class="size-4" />
							{t('organizations.servers.repair')}
						</DropdownMenu.Item>
						<DropdownMenu.Item onSelect={() => onMaintenance(server)}>
							<WrenchIcon class="size-4" />
							{t('organizations.servers.maintenance')}
						</DropdownMenu.Item>
						<DropdownMenu.Item onSelect={() => onShowKey(server)}>
							<KeyRoundIcon class="size-4" />
							{t('organizations.servers.show_public_key')}
						</DropdownMenu.Item>
						<DropdownMenu.Item onSelect={() => onRename(server)}>
							<PencilIcon class="size-4" />
							{t('organizations.servers.rename_action')}
						</DropdownMenu.Item>
						{#if groups && onToggleGroup}
							<!-- A server may sit in several groups, so each group toggles on its own. -->
							<DropdownMenu.Sub>
								<DropdownMenu.SubTrigger>
									<BoxesIcon class="size-4" />
									{t('organizations.server_groups.title')}
								</DropdownMenu.SubTrigger>
								<DropdownMenu.SubContent class="max-h-72 overflow-y-auto">
									{#each groups as group (group.id)}
										{@const member = group.member_servers.some((entry) => entry.id === server.id)}
										<DropdownMenu.CheckboxItem
											checked={member}
											onCheckedChange={(next) => onToggleGroup(server, group, next)}>
											<span class="truncate">{group.name}</span>
										</DropdownMenu.CheckboxItem>
									{/each}
									{#if groups.length}<DropdownMenu.Separator />{/if}
									<DropdownMenu.Item onSelect={() => onNewGroup?.(server)}>
										<PlusIcon class="size-4" />
										{t('organizations.server_groups.new_group')}
									</DropdownMenu.Item>
								</DropdownMenu.SubContent>
							</DropdownMenu.Sub>
						{/if}
						<DropdownMenu.Separator />
						<!-- Still selectable while applications are assigned: the dialog is where the
						     user finds out which ones have to move first. -->
						<DropdownMenu.Item variant="destructive" onSelect={() => onDelete(server)}>
							<Trash2Icon class="size-4" />
							{t('organizations.servers.remove')}
						</DropdownMenu.Item>
					</DropdownMenu.Content>
				</DropdownMenu.Root>
			{:else if canManage}
				<DropdownMenu.Root>
					<DropdownMenu.Trigger>
						{#snippet child({ props })}
							<Button
								{...props}
								size="icon-sm"
								variant="outline"
								aria-label={t('organizations.servers.actions')}>
								<EllipsisVerticalIcon class="size-4" />
							</Button>
						{/snippet}
					</DropdownMenu.Trigger>
					<DropdownMenu.Content align="end">
						<DropdownMenu.Item onSelect={() => onRename(server)}>
							<PencilIcon class="size-4" />
							{t('organizations.servers.rename_action')}
						</DropdownMenu.Item>
					</DropdownMenu.Content>
				</DropdownMenu.Root>
			{/if}
		</div>
	</div>

	{#if server.last_error}
		<p
			class="rounded-lg border border-destructive/30 bg-destructive/5 p-2 text-sm text-destructive">
			{server.last_error.message}
		</p>
	{/if}

	{#if history}
		<div class="@container">
			<div class="grid gap-4 @md:grid-cols-2 {hasDisk ? '@2xl:grid-cols-3' : ''}">
				{@render usage(
					t('servers.card.cpu_24h'),
					cpu,
					cpuSlots,
					percent(cpu.at(-1) ?? 0),
					(i) => percent(cpu[i]),
					Math.max(100, ...cpu),
					server.state === 'degraded' ? 'warning' : 'primary',
					(v) => `${percent(v)} CPU`,
				)}
				{@render usage(
					t('servers.card.ram_24h'),
					memSeries.values.map((v) => v ?? 0),
					memSeries.values,
					bytesLabel(memSeries.values, memCaps, lastIndex(memSeries.values)),
					(i) => bytesLabel(memSeries.values, memCaps, i),
					Math.max(peakOf(memCaps), peakOf(memSeries.values) * 1.08, 1),
					bytesTone(memSeries.values, memCaps),
					(v) => `${gib(v) ?? '0.0 GiB'} RAM`,
					memCaps,
				)}
				{#if hasDisk}
					<div class="min-w-0 @md:col-span-2 @2xl:col-span-1">
						{@render usage(
							t('servers.card.disk_24h'),
							diskSeries.values.map((v) => v ?? 0),
							diskSeries.values,
							bytesLabel(diskSeries.values, diskSeries.caps, lastIndex(diskSeries.values)),
							(i) => bytesLabel(diskSeries.values, diskSeries.caps, i),
							Math.max(peakOf(diskSeries.caps), peakOf(diskSeries.values), 1),
							bytesTone(diskSeries.values, diskSeries.caps) === 'warning' ? 'warning' : 'primary',
							(v) => `${gib(v) ?? '0.0 GiB'}`,
							diskSeries.caps,
						)}
					</div>
				{/if}
			</div>
		</div>
	{/if}

	{#if specs.length}
		<dl class="grid grid-cols-2 gap-2.5 {isRemote ? '' : 'sm:grid-cols-4'}">
			{#each specs as [key, value] (key)}
				<div class="flex min-w-0 flex-col gap-0.5 rounded-lg bg-muted/60 px-3 py-2.5 lp-inset">
					<dt class="font-mono text-[11.5px] text-muted-foreground">
						{t(`servers.card.spec_${key}`)}
					</dt>
					<dd class="truncate text-[13px] font-medium" title={value}>{value}</dd>
				</div>
			{/each}
		</dl>
	{/if}

	{#if isRemote && canManage && server.facts.helper?.outdated}
		<ServerHelperNotice
			organizationId={server.organization_id ?? getSelectedOrganizationId() ?? ''}
			{server}
			onUpdated={() => onHealthcheck(server)} />
	{/if}

	{#if isRemote && canManage && server.state === 'ready' && server.facts.swap_recommended}
		<ServerSwapNotice
			organizationId={server.organization_id ?? getSelectedOrganizationId() ?? ''}
			{server}
			onCreated={() => onHealthcheck(server)} />
	{/if}

	{#if server.host_key_fingerprint}
		<CopyButton
			value={server.host_key_fingerprint}
			label={server.host_key_fingerprint}
			copiedLabel={t('servers.card.fingerprint_copied')}
			class="max-w-full font-mono text-xs text-muted-foreground hover:text-foreground" />
	{/if}

	<div
		class="mt-auto flex flex-wrap gap-x-3.5 gap-y-1.5 pt-3.5 text-xs text-muted-foreground lp-groove-t [&_svg]:size-3.25">
		<span class="inline-flex items-center gap-1.5">
			<BoxIcon />
			{server.assigned_application_count}
			{t(
				server.assigned_application_count === 1
					? 'organizations.servers.application_assigned'
					: 'organizations.servers.applications_assigned',
			)}
		</span>
		<span class="inline-flex items-center gap-1.5">
			<NetworkIcon />{server.ingress_mode.replaceAll('_', ' ')}
		</span>
		{#if isRemote}
			<span class="inline-flex items-center gap-1.5"><LayersIcon />{server.podman_mode}</span>
		{/if}
	</div>
</Card.Root>
