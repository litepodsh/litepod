<script lang="ts">
	import SkeletonRows from '#lib/components/custom/skeleton-rows.svelte';
	import SoftCheckbox from '#lib/components/custom/soft-checkbox.svelte';
	import { onMount } from 'svelte';
	import { page } from '$app/state';
	import { goto } from '$app/navigation';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Card from '#lib/components/ui/card/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Label } from '#lib/components/ui/label/index.js';
	import * as Select from '#lib/components/ui/select/index.js';
	import * as AlertDialog from '#lib/components/ui/alert-dialog/index.js';
	import DeleteConfirmControl from '#lib/components/custom/delete-confirm-control.svelte';
	import * as Dialog from '#lib/components/ui/dialog/index.js';
	import * as DropdownMenu from '#lib/components/ui/dropdown-menu/index.js';
	import DebugJsonPanel from '#lib/components/debug-json-panel.svelte';
	import VolumeFileWorkspace from '#lib/components/volumes/volume-file-workspace.svelte';
	import { AnimatedTabs } from '#lib/components/custom/animated-tabs/index.js';
	import { toast } from '#lib/utils/toast.js';
	import {
		listServers,
		listServerPorts,
		type Server,
		type ServerPort,
	} from '#lib/clients/server-client.js';
	import { getSystemVersion } from '#lib/clients/system-client.js';
	import { serverGroups, deploymentTargetNotice } from '#lib/utils/server-visibility.js';
	import {
		containerAction,
		createNetwork,
		createVolume,
		deletePodmanResource,
		getContainerTop,
		getPodmanEvents,
		getPodmanOverview,
		getPodmanResource,
		getPodmanSystemDf,
		listPodmanResources,
		networkMember,
		podAction,
		pruneBuildCache,
		prunePodmanResource,
		pullImage,
		streamPodmanEvents,
		streamPodmanMetrics,
		volumeWorkspace,
		type ContainerAction,
		type PodmanRecord,
		type PodmanResource,
	} from '#lib/clients/podman-console-client.js';
	import {
		formatCpu,
		formatMemory,
		type ContainerMetric,
		type ContainerMetricsSnapshot,
	} from '#lib/clients/container-metrics-client.js';
	import RefreshCwIcon from '@lucide/svelte/icons/refresh-cw';
	import XIcon from '@lucide/svelte/icons/x';
	import Trash2Icon from '@lucide/svelte/icons/trash-2';
	import PlayIcon from '@lucide/svelte/icons/play';
	import SquareIcon from '@lucide/svelte/icons/square';
	import RotateCwIcon from '@lucide/svelte/icons/rotate-cw';
	import FileTextIcon from '@lucide/svelte/icons/file-text';
	import ActivityIcon from '@lucide/svelte/icons/activity';
	import FolderOpenIcon from '@lucide/svelte/icons/folder-open';
	import TerminalIcon from '@lucide/svelte/icons/terminal';
	import MoreHorizontalIcon from '@lucide/svelte/icons/more-horizontal';
	import ArrowDownUpIcon from '@lucide/svelte/icons/arrow-down-up';
	import ContainerShell from '#lib/components/containers/container-shell.svelte';
	import ContainerLogView from '#lib/components/containers/container-log-view.svelte';
	import ResourceGauge from '#lib/components/containers/resource-gauge.svelte';
	import {
		containerSizeBytes,
		resourcePercent,
		sortContainerUsage,
		type UsageSort,
	} from '#lib/utils/podman-monitoring.js';
	import { containerStatusTone } from '#lib/utils/container-status.js';
	import { toneBadge, toneDot } from '#lib/utils/status-tone.js';
	import { t } from '#lib/i18n/index.svelte.js';
	import { allowed } from '#lib/state/permissions.svelte.js';

	type Tab =
		| 'containers'
		| 'pods'
		| 'images'
		| 'volumes'
		| 'networks'
		| 'events'
		| 'disk-usage'
		| 'health'
		| 'monitoring';
	const tabs: { value: Tab; label: string }[] = [
		{ value: 'containers', label: t('organizations.containers.containers') },
		{ value: 'monitoring', label: t('organizations.containers.monitoring') },
		{ value: 'pods', label: t('organizations.containers.pods') },
		{ value: 'images', label: t('organizations.containers.images') },
		{ value: 'volumes', label: t('organizations.containers.volumes') },
		{ value: 'networks', label: t('organizations.containers.networks') },
		{ value: 'events', label: t('organizations.containers.events') },
		{ value: 'disk-usage', label: t('organizations.containers.disk_usage') },
		{ value: 'health', label: t('organizations.containers.health') },
	];
	const RESOURCE_TABS: Partial<Record<Tab, PodmanResource>> = {
		containers: 'containers',
		pods: 'pods',
		images: 'images',
		volumes: 'volumes',
		networks: 'networks',
	};
	function tabFromUrl(): Tab {
		// `tab` belongs to the System page's own section list, so this panel uses `view`.
		const requested = page.url.searchParams.get('view');
		return tabs.some((tab) => tab.value === requested) ? (requested as Tab) : 'containers';
	}

	let { organizationId }: { organizationId: string } = $props();
	const canManage = $derived(allowed('server.manage'));

	let servers = $state<Server[]>([]);
	let isCloud = $state(false);
	let serverId = $state('');
	let active = $state<Tab>(tabFromUrl());

	let entries = $state<PodmanRecord[]>([]);
	/** Host ports litepod tracks for this server (DB-backed, not raw Podman) - used to tag each container's published ports with who reserved them. */
	let ports = $state<ServerPort[]>([]);
	let selected = $state<{ id: string; data: PodmanRecord } | null>(null);
	type TermView = {
		mode: 'logs' | 'top' | 'shell';
		id: string;
		name: string;
		lines: string[];
		streaming: boolean;
	};
	let termView = $state<TermView | null>(null);
	let termBody = $state<HTMLElement | null>(null);
	let browsingVolume = $state<string | null>(null);
	let showDetailJson = $state(false);
	let overview = $state<PodmanRecord | null>(null);
	let diskUsage = $state<unknown>(null);
	let events = $state<PodmanRecord[]>([]);
	let showRaw = $state(false);
	let query = $state('');
	let loading = $state(false);
	let actionBusy = $state(false);
	let error = $state('');
	let metricsByContainer = $state<Map<string, ContainerMetric>>(new Map());
	let usageSort = $state<UsageSort>('cpu');

	let imageReference = $state('');
	let newVolumeName = $state('');
	let newVolumeDriver = $state('');

	let net = $state({
		name: '',
		driver: '',
		subnet: '',
		gateway: '',
		internal: false,
	});

	let pending = $state<{
		kind: 'container' | 'pod' | 'delete' | 'prune' | 'pull' | 'build-cache' | 'net-member';
		label: string;
		expected: string;
		run: () => Promise<unknown>;
	} | null>(null);

	let serverOptions = $derived.by(() => {
		const groups = serverGroups(servers, isCloud);
		return [...groups.local, ...groups.remote].filter((server) => server.state === 'ready');
	});
	let readyRemoteCount = $derived(
		serverGroups(servers, isCloud).remote.filter((s) => s.state === 'ready').length,
	);
	let targetNotice = $derived(deploymentTargetNotice(isCloud, readyRemoteCount));
	let activeServer = $derived(serverOptions.find((server) => server.id === serverId) ?? null);
	// Podman hands back whatever the host's version prints, so there is no schema
	// to search - but stringifying the whole record matches image SHAs, label
	// blobs and mount options, which makes a query like "web" hit unrelated rows.
	// Project the fields an operator actually searches by instead.
	function searchText(entry: PodmanRecord): string {
		return [
			entryName(entry),
			entryId(entry)
				.replace(/^sha256:/, '')
				.slice(0, 12),
			containerImage(entry),
			containerState(entry),
			summary(entry),
			String(entry.Driver ?? entry.driver ?? ''),
			String(entry.Mountpoint ?? entry.mountpoint ?? ''),
			...imageRepoTags(entry),
			...containerPorts(entry),
		]
			.join(' ')
			.toLowerCase();
	}

	// Built once per `entries` change, not once per keystroke.
	let searchIndex = $derived(entries.map((entry) => ({ entry, text: searchText(entry) })));
	let filtered = $derived.by(() => {
		const needle = query.trim().toLowerCase();
		if (!needle) return entries;
		return searchIndex.filter((row) => row.text.includes(needle)).map((row) => row.entry);
	});
	let monitorTiles = $derived.by(() => {
		const info = (overview as any)?.info;
		const host = info?.host;
		const store = info?.store;
		const memoryTotal = host?.memTotal as number | undefined;
		const memoryAvailable = (host?.memAvailable ?? host?.memFree) as number | undefined;
		const memoryUsed =
			memoryTotal !== undefined && memoryAvailable !== undefined
				? memoryTotal - memoryAvailable
				: undefined;
		return [
			{
				key: 'cpu',
				label: t('organizations.containers.cpu_label'),
				percent:
					host?.cpuUtilization?.idlePercent === undefined
						? null
						: Math.max(0, Math.min(100, 100 - host.cpuUtilization.idlePercent)),
				detail: host?.cpus
					? t('organizations.containers.vcpus', { count: host.cpus })
					: t('organizations.containers.podman_host'),
			},
			{
				key: 'memory',
				label: t('organizations.containers.memory'),
				percent: resourcePercent(memoryUsed, memoryTotal),
				detail:
					memoryUsed === undefined || memoryTotal === undefined
						? t('organizations.containers.unavailable')
						: `${formatBytes(memoryUsed)} / ${formatBytes(memoryTotal)}`,
			},
			{
				key: 'storage',
				label: t('organizations.containers.storage'),
				percent: resourcePercent(store?.graphRootUsed, store?.graphRootAllocated),
				detail:
					store?.graphRootAllocated === undefined
						? t('organizations.containers.unavailable')
						: `${formatBytes(store?.graphRootUsed)} / ${formatBytes(store?.graphRootAllocated)}`,
			},
		];
	});
	let monitorUsage = $derived.by(() =>
		sortContainerUsage(
			entries.map((entry) => {
				const metric =
					metricsByContainer.get(entryId(entry)) ?? metricsByContainer.get(entryName(entry));
				return {
					id: entryId(entry),
					name: entryName(entry),
					cpu: metric?.cpu_percent ?? null,
					memory: metric?.memory_usage_bytes ?? null,
					disk: containerSizeBytes(entry.Size ?? entry.size ?? entry.SizeRw ?? entry.sizeRw),
				};
			}),
			usageSort,
		),
	);

	function entryId(entry: PodmanRecord) {
		const candidate =
			entry.Id ?? entry.id ?? entry.Name ?? entry.name ?? entry.Names ?? entry.names;
		return Array.isArray(candidate) ? String(candidate[0] ?? '') : String(candidate ?? '');
	}
	function entryName(entry: PodmanRecord) {
		const candidate =
			entry.Names ?? entry.names ?? entry.Name ?? entry.name ?? entry.Id ?? entry.id;
		return Array.isArray(candidate)
			? String(candidate[0] ?? entryId(entry))
			: String(candidate ?? entryId(entry));
	}
	function summary(entry: PodmanRecord) {
		return String(
			entry.Status ??
				entry.status ??
				entry.State ??
				entry.state ??
				entry.Created ??
				entry.created ??
				'',
		);
	}
	function containerState(entry: PodmanRecord) {
		return String(entry.State ?? entry.state ?? 'unknown');
	}
	function containerStateClass(entry: PodmanRecord) {
		return toneBadge(containerStatusTone(containerState(entry)));
	}

	function formatBytes(value: unknown): string {
		const n = typeof value === 'string' ? Number(value) : (value as number);
		if (!Number.isFinite(n)) return String(value ?? '—');
		const units = ['B', 'KiB', 'MiB', 'GiB', 'TiB'];
		let v = n;
		let u = 0;
		while (v >= 1024 && u < units.length - 1) {
			v /= 1024;
			u += 1;
		}
		return `${v >= 10 || Number.isInteger(v) ? v.toFixed(0) : v.toFixed(1)} ${units[u]}`;
	}

	function createdMs(value: unknown): number | null {
		if (typeof value === 'number') return value > 1e12 ? value : value * 1000;
		if (typeof value === 'string') {
			const t = Date.parse(value);
			return Number.isFinite(t) ? t : null;
		}
		return null;
	}
	function relativeAge(ms: number | null): string {
		if (ms === null) return '';
		const diff = Date.now() - ms;
		if (diff < 0) return t('organizations.containers.just_now');
		const units: [number, string][] = [
			[31536e6, 'y'],
			[2592e6, 'mo'],
			[864e5, 'd'],
			[36e5, 'h'],
			[6e4, 'm'],
		];
		for (const [u, label] of units)
			if (diff >= u)
				return t('organizations.containers.ago', { value: `${Math.floor(diff / u)}${label}` });
		return t('organizations.containers.just_now');
	}

	// ---- images ----
	function imageRepoTags(entry: PodmanRecord): string[] {
		const tags = entry.RepoTags ?? entry.repoTags ?? entry.Names ?? entry.names;
		return (Array.isArray(tags) ? tags.map(String) : []).filter((t) => t && t !== '<none>:<none>');
	}
	function imageDangling(entry: PodmanRecord): boolean {
		return entry.Dangling === true || entry.dangling === true || imageRepoTags(entry).length === 0;
	}
	function imageSize(entry: PodmanRecord): string {
		return formatBytes(entry.Size ?? entry.size ?? entry.VirtualSize ?? entry.virtualSize);
	}
	function imageShortId(entry: PodmanRecord): string {
		return entryId(entry)
			.replace(/^sha256:/, '')
			.slice(0, 12);
	}
	function imageInUse(entry: PodmanRecord): number {
		return Number(entry.Containers ?? entry.containers ?? 0) || 0;
	}

	// ---- containers ----
	function containerImage(entry: PodmanRecord): string {
		return String(entry.Image ?? entry.image ?? entry.ImageName ?? '');
	}
	function containerPorts(entry: PodmanRecord): string[] {
		const raw = entry.Ports ?? entry.ports;
		if (!Array.isArray(raw)) return [];
		const out = raw.map((port) => {
			if (typeof port !== 'object' || port === null) return String(port);
			const p = port as Record<string, unknown>;
			const hostPort = p.host_port ?? p.HostPort ?? p.PublicPort;
			const cPort = p.container_port ?? p.ContainerPort ?? p.PrivatePort;
			const proto = String(p.protocol ?? p.Type ?? 'tcp');
			if (cPort === undefined || cPort === null) return '';
			return hostPort ? `${hostPort}→${cPort}/${proto}` : `${cPort}/${proto}`;
		});
		return [...new Set(out.filter(Boolean))];
	}
	/** litepod's own published-port bookkeeping for this container's host ports (matched by port number - unique per server, so exact without needing to parse compose labels). */
	function trackedPorts(entry: PodmanRecord): ServerPort[] {
		const raw = entry.Ports ?? entry.ports;
		if (!Array.isArray(raw)) return [];
		const hostPorts = new Set(
			raw
				.map((port) => {
					if (typeof port !== 'object' || port === null) return '';
					const p = port as Record<string, unknown>;
					return String(p.host_port ?? p.HostPort ?? p.PublicPort ?? '');
				})
				.filter(Boolean),
		);
		return ports.filter((p) => hostPorts.has(p.host_port));
	}

	type KV = [string, string];
	let detailRows = $derived.by<KV[]>(() => {
		if (!selected) return [];
		const d = selected.data as any;
		const clean = (rows: [string, unknown][]): KV[] =>
			rows
				.filter(
					([, v]) => v !== undefined && v !== null && v !== '' && !(Array.isArray(v) && !v.length),
				)
				.map(([k, v]) => [k, Array.isArray(v) ? v.join(', ') : String(v)] as KV);
		if (active === 'images') {
			const cfg = d.Config ?? {};
			return clean([
				['Tags', d.RepoTags],
				['Digest', d.RepoDigests?.[0] ?? d.Digest],
				['Size', formatBytes(d.Size ?? d.VirtualSize)],
				['Created', d.Created ? new Date(d.Created).toLocaleString() : ''],
				['Platform', [d.Os, d.Architecture].filter(Boolean).join('/')],
				['Entrypoint', (cfg.Entrypoint ?? []).join(' ')],
				['Cmd', (cfg.Cmd ?? []).join(' ')],
				['Workdir', cfg.WorkingDir],
				['Exposed ports', Object.keys(cfg.ExposedPorts ?? {})],
				['Env vars', (cfg.Env ?? []).length || ''],
			]);
		}
		if (active === 'containers') {
			const state = d.State ?? {};
			const cfg = d.Config ?? {};
			const net = d.NetworkSettings ?? {};
			const mounts = (d.Mounts ?? []).map(
				(m: any) =>
					`${m.Source ?? m.Name ?? '?'} → ${m.Destination} (${m.RW === false ? 'ro' : 'rw'})`,
			);
			const ports = Object.entries<any>(net.Ports ?? {})
				.filter(([, v]) => Array.isArray(v) && v.length)
				.map(([k, v]) => `${v[0].HostPort}→${k}`);

			return clean([
				['Image', d.ImageName ?? d.Image],
				['Status', state.Status],
				['Health', state.Health?.Status],
				['Started', state.StartedAt ? new Date(state.StartedAt).toLocaleString() : ''],
				['Exit code', state.Running ? '' : state.ExitCode],
				['Restarts', d.RestartCount],
				['Created', d.Created ? new Date(d.Created).toLocaleString() : ''],
				['Networks', Object.keys(net.Networks ?? {})],
				['Ports', ports],
				['Mounts', mounts],
				['Cmd', (cfg.Cmd ?? []).join(' ')],
				['Workdir', cfg.WorkingDir],
				['Env vars', (cfg.Env ?? []).length || ''],
			]);
		}
		return [];
	});

	type DfCard = { title: string; rows: [string, string][] };
	let dfCards = $derived.by<DfCard[]>(() => {
		const df = diskUsage as any;
		if (Array.isArray(df)) {
			return df.map((row) => ({
				title: String(row.Type ?? row.type ?? 'Section'),
				rows: [
					['Total', String(row.Total ?? row.TotalCount ?? '—')],
					['Active', String(row.Active ?? row.ActiveCount ?? '—')],
					['Size', typeof row.Size === 'number' ? formatBytes(row.Size) : String(row.Size ?? '—')],

					[
						'Reclaimable',
						typeof row.Reclaimable === 'number'
							? formatBytes(row.Reclaimable)
							: String(row.Reclaimable ?? '—'),
					],
				],
			}));
		}
		if (df && typeof df === 'object') {
			return Object.entries(df)
				.filter(([, v]) => Array.isArray(v))
				.map(([key, v]) => {
					const arr = v as any[];
					const size = arr.reduce((sum, item) => sum + (Number(item.Size ?? item.size) || 0), 0);
					return {
						title: key,
						rows: [
							['Count', String(arr.length)],
							['Size', formatBytes(size)],
						],
					};
				});
		}
		return [];
	});

	type HealthCard = { title: string; rows: [string, string][] };
	let healthCards = $derived.by<HealthCard[]>(() => {
		const ov = overview as any;
		if (!ov) return [];
		const info = ov.info ?? {};
		const host = info.host ?? {};
		const store = info.store ?? {};
		const version = ov.version ?? info.version ?? {};
		const clean = (rows: [string, unknown][]): [string, string][] =>
			rows
				.filter(([, v]) => v !== undefined && v !== null && v !== '')
				.map(([k, v]) => [k, String(v)] as [string, string]);
		return [
			{
				title: 'Host',
				rows: clean([
					['OS', host.os],
					['Architecture', host.arch],
					['Kernel', host.kernel],
					['Hostname', host.hostname],
					['CPUs', host.cpus],
				]),
			},
			{
				title: 'Podman',
				rows: clean([
					['Version', version.Version ?? version.version],
					['API version', version.APIVersion],
					['Go version', version.GoVersion],
					['Built', version.BuiltTime],
				]),
			},
			{
				title: 'Runtime',
				rows: clean([
					['OCI runtime', host.ociRuntime?.name],
					['Rootless', host.security?.rootless],
					['cgroup version', host.cgroupVersion],
					['Events logger', host.eventLogger],
				]),
			},
			{
				title: 'Storage',
				rows: clean([
					['Driver', store.graphDriverName],
					['Graph root', store.graphRoot],
					['Run root', store.runRoot],
					['Containers', store.containerStore?.number],
					['Images', store.imageStore?.number],
				]),
			},
		].filter((card) => card.rows.length > 0);
	});

	function setServerQuery(id: string) {
		const url = new URL(page.url.href);
		url.searchParams.set('server', id);
		void goto(`${url.pathname}${url.search}`, {
			replace: true,
			reset: false,
		});
	}
	function setTabQuery(tab: Tab) {
		const url = new URL(page.url.href);
		url.searchParams.set('view', tab);
		goto(url, { shallow: true, replace: true, state: page.state });
	}

	function resetDetail() {
		selected = null;
		termView = null;
		browsingVolume = null;
	}

	async function loadTab() {
		if (!serverId) return;
		loading = true;
		error = '';
		resetDetail();
		try {
			const resource = RESOURCE_TABS[active];
			if (resource === 'containers') {
				[entries, ports] = await Promise.all([
					listPodmanResources(organizationId, serverId, resource),
					listServerPorts(organizationId, serverId).catch(() => [] as ServerPort[]),
				]);
			} else if (resource) {
				entries = await listPodmanResources(organizationId, serverId, resource);
			} else if (active === 'events') {
				events = await getPodmanEvents(organizationId, serverId);
			} else if (active === 'disk-usage') {
				diskUsage = await getPodmanSystemDf(organizationId, serverId);
			} else if (active === 'health') {
				overview = await getPodmanOverview(organizationId, serverId);
			} else if (active === 'monitoring') {
				const [nextOverview, containers] = await Promise.all([
					getPodmanOverview(organizationId, serverId),
					listPodmanResources(organizationId, serverId, 'containers'),
				]);
				overview = nextOverview;
				entries = containers;
			}
		} catch (err) {
			error = err instanceof Error ? err.message : t('organizations.containers.reach_failed');
		} finally {
			loading = false;
		}
	}

	async function changeServer(id: string) {
		serverId = id;
		setServerQuery(id);
		await loadTab();
	}
	async function changeTab(value: string) {
		active = value as Tab;
		setTabQuery(active);
		query = '';
		await loadTab();
	}

	async function inspect(entry: PodmanRecord) {
		const resource = RESOURCE_TABS[active];
		const id = entryId(entry);
		if (!resource || !id) return;
		termView = null;
		browsingVolume = null;
		showDetailJson = false;
		try {
			selected = { id, data: await getPodmanResource(organizationId, serverId, resource, id) };
		} catch (err) {
			toast.error(
				err instanceof Error ? err.message : t('organizations.containers.inspect_failed'),
			);
		}
	}
	async function inspectAndAskContainerAction(entry: PodmanRecord) {
		await inspect(entry);
		if (selected?.id === entryId(entry)) {
			askContainerAction(containerState(entry).toLowerCase() === 'running' ? 'stop' : 'start');
		}
	}
	async function openContainerFromMonitoring(id: string) {
		const entry = entries.find((item) => entryId(item) === id);
		if (!entry) return;
		active = 'containers';
		query = '';
		await inspect(entry);
	}

	function openLogs() {
		if (!selected) return;
		// The log view owns its own fetch/stream/toolbar.
		termView = {
			mode: 'logs',
			id: selected.id,
			name: entryName(selected.data),
			lines: [],
			streaming: true,
		};
	}
	function openTop() {
		if (!selected) return;
		termView = {
			mode: 'top',
			id: selected.id,
			name: entryName(selected.data),
			lines: [t('common.common.loading')],
			streaming: false,
		};
	}
	function openShell() {
		if (!selected) return;
		termView = {
			mode: 'shell',
			id: selected.id,
			name: entryName(selected.data),
			lines: [],
			streaming: false,
		};
	}
	// Poll `podman top` while the top view is open. Keyed on the container id via
	// $derived so the per-tick `lines` write doesn't retrigger this effect and
	// restart the interval (that feedback loop polled as fast as the network
	// allowed and tripped the API rate limiter with "Too many requests").
	const topTargetId = $derived(termView?.mode === 'top' ? termView.id : null);
	$effect(() => {
		if (!topTargetId || !serverId) return;
		const current = topTargetId;
		let busy = false;
		const tick = async () => {
			if (busy) return;
			busy = true;
			try {
				const { processes } = await getContainerTop(organizationId, serverId, current);
				if (termView?.id === current && termView.mode === 'top')
					termView = {
						...termView,
						lines: (processes || t('organizations.containers.log_lines_placeholder')).split('\n'),
					};
			} catch (err) {
				if (termView?.id === current && termView.mode === 'top')
					termView = {
						...termView,
						lines: [err instanceof Error ? err.message : 'top failed'],
					};
			} finally {
				busy = false;
			}
		};
		void tick();
		const timer = setInterval(tick, 1000);
		return () => clearInterval(timer);
	});

	// Keep the terminal pane scrolled to the newest line.
	$effect(() => {
		termView?.lines.length;
		if (termBody) termBody.scrollTop = termBody.scrollHeight;
	});

	function askContainerAction(action: ContainerAction) {
		if (!selected) return;
		pending = {
			kind: 'container',
			label: t('organizations.containers.action_container', {
				action: t(`organizations.containers.${action === 'unpause' ? 'resume' : action}`),
			}),
			expected: selected.id,
			run: () => containerAction(organizationId, serverId, selected!.id, action, selected!.id),
		};
	}
	function askPodAction(action: 'start' | 'stop' | 'restart' | 'kill') {
		if (!selected) return;
		pending = {
			kind: 'pod',
			label: t('organizations.containers.action_pod', {
				action: t(`organizations.containers.${action}`),
			}),
			expected: selected.id,
			run: () => podAction(organizationId, serverId, selected!.id, action, selected!.id),
		};
	}
	function askDelete() {
		const resource = RESOURCE_TABS[active];
		if (!selected || !resource) return;
		const target = entryName(selected.data);
		pending = {
			kind: 'delete',
			label: t(`organizations.containers.delete_${resource.slice(0, -1)}`),
			expected: target,
			run: () => deletePodmanResource(organizationId, serverId, resource, target, target),
		};
	}
	function askPrune(resource: PodmanResource) {
		pending = {
			kind: 'prune',
			label: t('organizations.containers.prune_unused_resource', {
				resource: t(`organizations.containers.${resource}`),
			}),
			expected: 'prune',
			run: () => prunePodmanResource(organizationId, serverId, resource, 'prune'),
		};
	}
	function askPruneBuildCache() {
		pending = {
			kind: 'build-cache',
			label: t('organizations.containers.prune_build_cache'),
			expected: 'prune',
			run: () => pruneBuildCache(organizationId, serverId, 'prune'),
		};
	}
	function askPull() {
		if (!imageReference.trim()) return;
		const ref = imageReference.trim();
		pending = {
			kind: 'pull',
			label: t('organizations.containers.pull_image'),
			expected: ref,
			run: () => pullImage(organizationId, serverId, ref, ref),
		};
	}
	function askNetworkMember(action: 'connect' | 'disconnect') {
		if (!selected) return;
		const container = window
			.prompt(
				t(
					`organizations.containers.${action === 'connect' ? 'prompt_connect' : 'prompt_disconnect'}`,
				),
			)
			?.trim();
		if (!container) return;
		pending = {
			kind: 'net-member',
			label: t(
				`organizations.containers.${action === 'connect' ? 'connect_to' : 'disconnect_from'}`,
				{ container },
			),
			expected: container,
			run: () => networkMember(organizationId, serverId, selected!.id, action, container),
		};
	}

	async function confirmPending() {
		if (!pending) return;
		actionBusy = true;
		try {
			await pending.run();
			toast.success(t('organizations.common.done'));
			await loadTab();
		} catch (err) {
			toast.error(
				err instanceof Error ? err.message : t('organizations.containers.operation_failed'),
			);
		} finally {
			actionBusy = false;
			pending = null;
		}
	}

	async function submitVolume(event: SubmitEvent) {
		event.preventDefault();
		if (!newVolumeName.trim()) return;
		actionBusy = true;
		try {
			await createVolume(organizationId, serverId, {
				name: newVolumeName.trim(),
				driver: newVolumeDriver.trim() || undefined,
			});
			newVolumeName = '';
			newVolumeDriver = '';
			toast.success(t('organizations.containers.volume_created'));
			await loadTab();
		} catch (err) {
			toast.error(
				err instanceof Error ? err.message : t('organizations.containers.volume_create_failed'),
			);
		} finally {
			actionBusy = false;
		}
	}
	async function submitNetwork(event: SubmitEvent) {
		event.preventDefault();
		if (!net.name.trim()) return;
		actionBusy = true;
		try {
			await createNetwork(organizationId, serverId, {
				name: net.name.trim(),
				driver: net.driver.trim() || undefined,
				subnet: net.subnet.trim() || undefined,
				gateway: net.gateway.trim() || undefined,
				internal: net.internal,
			});
			net = { name: '', driver: '', subnet: '', gateway: '', internal: false };
			toast.success(t('organizations.containers.network_created'));
			await loadTab();
		} catch (err) {
			toast.error(
				err instanceof Error ? err.message : t('organizations.containers.network_create_failed'),
			);
		} finally {
			actionBusy = false;
		}
	}

	$effect(() => {
		if (!['containers', 'monitoring'].includes(active) || !serverId) {
			metricsByContainer = new Map();
			return;
		}
		const stream = streamPodmanMetrics(organizationId, serverId);
		const onMetrics = (event: Event) => {
			try {
				const snapshot = JSON.parse(
					(event as MessageEvent<string>).data,
				) as ContainerMetricsSnapshot;
				const map = new Map<string, ContainerMetric>();
				for (const metric of snapshot.metrics) {
					map.set(metric.id, metric);
					map.set(metric.name, metric);
				}
				metricsByContainer = map;
			} catch {
				// keep last good metrics
			}
		};
		stream.addEventListener('metrics', onMetrics);
		return () => {
			stream.removeEventListener('metrics', onMetrics);
			stream.close();
		};
	});

	$effect(() => {
		if (active !== 'monitoring' || !serverId) return;
		let busy = false;
		const tick = async () => {
			if (busy) return;
			busy = true;
			try {
				overview = await getPodmanOverview(organizationId, serverId);
			} catch {
				// Keep the last good host reading while the next poll retries.
			} finally {
				busy = false;
			}
		};
		void tick();
		const timer = setInterval(tick, 2_000);
		return () => clearInterval(timer);
	});

	$effect(() => {
		if (active !== 'events' || !serverId) return;
		const stream = streamPodmanEvents(organizationId, serverId);
		const onEvent = (event: Event) => {
			try {
				const parsed = JSON.parse((event as MessageEvent<string>).data) as PodmanRecord;
				events = [parsed, ...events].slice(0, 200);
			} catch {
				// ignore malformed line
			}
		};
		stream.addEventListener('event', onEvent);
		return () => {
			stream.removeEventListener('event', onEvent);
			stream.close();
		};
	});

	onMount(async () => {
		try {
			const [serverList, version] = await Promise.all([
				listServers(organizationId),
				getSystemVersion().catch(() => ({ is_cloud: false }) as { is_cloud: boolean }),
			]);
			servers = serverList;
			isCloud = version.is_cloud;
			const fromQuery = page.url.searchParams.get('server');
			active = tabFromUrl();
			serverId =
				(fromQuery && serverOptions.some((s) => s.id === fromQuery) && fromQuery) ||
				serverOptions[0]?.id ||
				'';

			if (serverId) await loadTab();
		} catch (err) {
			error = err instanceof Error ? err.message : t('organizations.common.servers_load_failed');
		}
	});
</script>

<div class="flex flex-col gap-4">
	<div class="flex flex-wrap items-end justify-between gap-3">
		<div class="min-w-0">
			<h2 class="text-lg font-semibold tracking-tight">{t('organizations.containers.title')}</h2>
			<p class="mt-1 text-sm text-muted-foreground">
				{t('organizations.containers.description')}
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
									? t('organizations.common.local')
									: t('organizations.common.remote')}</span>
						</Select.Item>
					{/each}
				</Select.Content>
			</Select.Root>
			<Button variant="outline" onclick={loadTab} disabled={loading || actionBusy || !serverId}>
				<RefreshCwIcon class={loading ? 'animate-spin' : ''} />{t('organizations.common.refresh')}
			</Button>
		</div>
	</div>

	{#if serverOptions.length === 0}
		<div
			class="rounded-2xl border border-dashed border-foreground/15 bg-card p-8 text-center text-sm text-muted-foreground">
			{targetNotice ?? t('organizations.common.no_ready_servers')}
		</div>
	{:else}
		{#if error}
			<div class="rounded-2xl border border-destructive/30 bg-card p-4 text-sm text-destructive">
				{error}
			</div>
		{/if}

		<Card.Root>
			<Card.Header class="gap-4">
				<div class="flex flex-wrap items-center justify-between gap-3">
					<div>
						<Card.Title>{tabs.find((t) => t.value === active)?.label}</Card.Title>
						<Card.Description>
							{#if RESOURCE_TABS[active]}{t('organizations.containers.shown', {
									count: filtered.length,
								})}{:else if active === 'events'}{t('organizations.containers.events_count', {
									count: events.length,
								})}{:else}{t('organizations.containers.live_data')}{/if}
						</Card.Description>
					</div>
					{#if active === 'images' && canManage}
						<form
							class="flex flex-wrap gap-2"
							onsubmit={(e) => {
								e.preventDefault();
								askPull();
							}}>
							<Input
								bind:value={imageReference}
								placeholder="registry/image:tag"
								class="w-full sm:w-56" />

							<Button type="submit" disabled={actionBusy || !imageReference}
								>{t('organizations.containers.pull')}</Button>

							<Button
								type="button"
								variant="outline"
								onclick={() => askPrune('images')}
								disabled={actionBusy}>{t('organizations.containers.prune')}</Button>
						</form>
					{/if}
				</div>
				<AnimatedTabs bind:value={active} onValueChange={changeTab} items={tabs} />
			</Card.Header>

			<Card.Content class="flex flex-col gap-5">
				{#if active === 'volumes' && canManage}
					<form
						class="lp-inset flex flex-wrap items-end gap-2 rounded-xl bg-muted/50 p-3"
						onsubmit={submitVolume}>
						<div class="grid gap-1">
							<Label class="text-xs">{t('organizations.common.name')}</Label>
							<Input bind:value={newVolumeName} placeholder="volume-name" class="w-48" />
						</div>
						<div class="grid gap-1">
							<Label class="text-xs">{t('organizations.containers.driver_optional')}</Label>
							<Input bind:value={newVolumeDriver} placeholder="local" class="w-40" />
						</div>

						<Button type="submit" disabled={actionBusy || !newVolumeName}
							>{t('organizations.containers.create_volume')}</Button>

						<Button
							type="button"
							variant="outline"
							onclick={() => askPrune('volumes')}
							disabled={actionBusy}>{t('organizations.containers.prune_unused')}</Button>
					</form>
				{:else if active === 'networks' && canManage}
					<form
						class="lp-inset grid gap-3 rounded-xl bg-muted/50 p-3 sm:grid-cols-2 lg:grid-cols-3"
						onsubmit={submitNetwork}>
						<div class="grid gap-1">
							<Label class="text-xs">{t('organizations.common.name')}</Label>
							<Input bind:value={net.name} placeholder="my-network" />
						</div>
						<div class="grid gap-1">
							<Label class="text-xs">{t('organizations.containers.driver')}</Label>

							<Select.Root type="single" value={net.driver} onValueChange={(v) => (net.driver = v)}>
								<Select.Trigger aria-label={t('organizations.containers.driver')}
									>{net.driver || 'bridge (default)'}</Select.Trigger>

								<Select.Content>
									<Select.Item value="" label="bridge (default)">bridge (default)</Select.Item>
									<Select.Item value="macvlan" label="macvlan">macvlan</Select.Item>
									<Select.Item value="ipvlan" label="ipvlan">ipvlan</Select.Item>
								</Select.Content>
							</Select.Root>
						</div>
						<div class="grid gap-1">
							<Label class="text-xs">{t('organizations.containers.subnet_optional')}</Label>
							<Input bind:value={net.subnet} placeholder="10.89.0.0/24" />
						</div>
						<div class="grid gap-1">
							<Label class="text-xs">{t('organizations.containers.gateway_optional')}</Label>
							<Input bind:value={net.gateway} placeholder="10.89.0.1" />
						</div>
						<label class="flex items-center gap-2 self-end text-sm">
							<SoftCheckbox bind:checked={net.internal} />
							{t('organizations.containers.internal')}
						</label>
						<div class="flex items-end gap-2">
							<Button type="submit" disabled={actionBusy || !net.name}
								>{t('organizations.containers.create_network')}</Button>
							<Button
								type="button"
								variant="outline"
								onclick={() => askPrune('networks')}
								disabled={actionBusy}>{t('organizations.containers.prune')}</Button>
						</div>
					</form>
				{/if}

				{#if loading}
					<SkeletonRows rows={5} height="h-14" label={t('organizations.containers.loading')} />
				{:else if RESOURCE_TABS[active]}
					<div class="grid gap-5 lg:grid-cols-[minmax(0,1fr)_minmax(18rem,0.9fr)]">
						<div class="min-w-0">
							<Input
								bind:value={query}
								placeholder={t('organizations.containers.search', { resource: active })}
								class="mb-3" />
							{#if filtered.length === 0}
								<div class="rounded-2xl border border-dashed border-foreground/15 p-8 text-center text-sm text-muted-foreground">
									{t('organizations.containers.empty_resources', { resource: active })}
								</div>
							{:else}
								<div class="lp-raised overflow-hidden rounded-2xl border bg-card">
									{#each filtered as entry, i (entryId(entry) + ':' + i)}
										{@const metric =
											metricsByContainer.get(entryId(entry)) ??
											metricsByContainer.get(entryName(entry))}
										<div
											class="flex items-start justify-between gap-2 border-b px-3 py-2 last:border-b-0 hover:bg-muted/60"
											class:bg-muted={selected?.id === entryId(entry)}>
											<button
												type="button"
												class="min-w-0 flex-1 px-1 py-1 text-left"
												onclick={() => inspect(entry)}>
												{#if active === 'images'}
													{@const tags = imageRepoTags(entry)}
													<div class="min-w-0 flex-1">
														<div class="flex flex-wrap items-center gap-1.5">
															{#if tags.length === 0}
																<span class="font-medium text-muted-foreground"><none></none></span>
																<span
																	class={`rounded-full border px-2 py-0.5 text-[11px] font-medium ${toneBadge('warn')}`}
																	>dangling</span>
															{:else}
																<span class="truncate font-medium">{tags[0]}</span>
																{#if tags.length > 1}<span class="text-[11px] text-muted-foreground"
																		>+{tags.length - 1}</span
																	>{/if}
															{/if}
														</div>
														<p class="mt-0.5 truncate font-mono text-xs text-muted-foreground">
															{imageShortId(entry)}
														</p>
													</div>
													<div class="flex shrink-0 flex-col items-end gap-0.5 text-xs">
														<span class="font-semibold tabular-nums">{imageSize(entry)}</span>
														<span class="text-muted-foreground"
															>{relativeAge(
																createdMs(entry.Created ?? entry.created ?? entry.CreatedAt),
															)}</span>
														{#if imageInUse(entry) > 0}
															<span class="text-success text-[11px]"
																>{t('organizations.containers.in_use', {
																	count: imageInUse(entry),
																})}</span>
														{/if}
													</div>
												{:else if active === 'containers'}
													{@const containerPortList = containerPorts(entry)}
													{@const hostPorts = trackedPorts(entry)}
													<div class="min-w-0 flex-1">
														<div class="flex items-center gap-2">
															<span
																class={`size-2 shrink-0 rounded-full ${toneDot(containerStatusTone(containerState(entry)))}`}
															></span>
															<span class="truncate font-medium">{entryName(entry)}</span>
														</div>
														<p class="mt-0.5 truncate text-xs text-muted-foreground">
															{containerImage(entry)}
														</p>
														{#if containerPortList.length}
															<div class="mt-1 flex flex-wrap gap-1">
																{#each containerPortList.slice(0, 4) as port (port)}
																	<span
																		class="lp-raised rounded-md bg-muted px-1.5 py-0.5 font-mono text-[10px] text-muted-foreground"
																		>{port}</span>
																{/each}
																{#if containerPortList.length > 4}<span
																		class="text-[10px] text-muted-foreground"
																		>+{containerPortList.length - 4}</span
																	>{/if}
															</div>
														{/if}
														{#if hostPorts.length}
															<div class="mt-1 flex flex-wrap gap-1">
																{#each hostPorts as p (p.host_port + p.source)}
																	<span
																		class="rounded border border-primary/30 bg-primary/5 px-1.5 py-0.5 font-mono text-[10px] text-primary"
																		title={t('organizations.containers.host_port_reserved', {
																			application: p.application_name,
																			source: p.source,
																		})}>{p.host_port}→{p.container_port}</span>
																{/each}
															</div>
														{/if}
													</div>
													<div class="flex shrink-0 flex-col items-end gap-0.5 text-xs">
														<span
															class={`rounded-full border px-2 py-0.5 text-[11px] font-medium capitalize ${containerStateClass(entry)}`}
															>{containerState(entry)}</span>
														{#if containerState(entry).toLowerCase() === 'running'}
															<span class="text-[11px] text-muted-foreground tabular-nums">
																{metric
																	? `${formatCpu(metric.cpu_percent)} · ${formatMemory(metric.memory_usage_bytes, metric.memory_limit_bytes)}`
																	: '—'}
															</span>
														{/if}
														<span class="text-[11px] text-muted-foreground"
															>{relativeAge(createdMs(entry.Created ?? entry.created))}</span>
													</div>
												{:else}
													<div class="min-w-0 flex-1">
														<p class="truncate font-medium">{entryName(entry)}</p>
														<p class="truncate font-mono text-xs text-muted-foreground">
															{entryId(entry)}
														</p>
													</div>
													<span
														class="shrink-0 max-w-[12rem] truncate text-xs text-muted-foreground"
														>{summary(entry)}</span>
												{/if}
											</button>
											{#if active === 'containers' && canManage}
												<Button
													size="sm"
													variant={containerState(entry).toLowerCase() === 'running'
														? 'outline'
														: 'default'}
													onclick={() => void inspectAndAskContainerAction(entry)}
													disabled={actionBusy}>
													{containerState(entry).toLowerCase() === 'running'
														? t('organizations.containers.stop')
														: t('organizations.containers.start')}
												</Button>
											{/if}
										</div>
									{/each}
								</div>
							{/if}
						</div>

						<div class="min-w-0">
							{#if selected}
								<div class="mb-3 flex flex-wrap gap-2">
									{#if active === 'containers'}
										{#if canManage}
											<Button
												size="sm"
												variant={containerState(selected.data).toLowerCase() === 'running'
													? 'outline'
													: 'default'}
												onclick={() =>
													selected &&
													askContainerAction(
														containerState(selected.data).toLowerCase() === 'running'
															? 'stop'
															: 'start',
													)}
												disabled={actionBusy}>
												{#if containerState(selected.data).toLowerCase() === 'running'}
													<SquareIcon />{t('organizations.containers.stop')}
												{:else}
													<PlayIcon />{t('organizations.containers.start')}
												{/if}
											</Button>
											<Button
												size="sm"
												variant="outline"
												onclick={() => askContainerAction('restart')}
												disabled={actionBusy}>
												<RotateCwIcon />
												{t('organizations.containers.restart')}
											</Button>
										{/if}

										<Button size="sm" variant="outline" onclick={openLogs} disabled={actionBusy}>
											<FileTextIcon />
											{t('organizations.containers.logs')}
										</Button>

										<Button size="sm" variant="outline" onclick={openTop} disabled={actionBusy}>
											<ActivityIcon />
											{t('organizations.containers.top')}
										</Button>

										{#if allowed('server.terminal')}
											<Button size="sm" variant="outline" onclick={openShell} disabled={actionBusy}>
												<TerminalIcon />
												{t('organizations.containers.shell')}
											</Button>
										{/if}

										{#if canManage}
											<DropdownMenu.Root>
												<DropdownMenu.Trigger>
													{#snippet child({ props })}
														<Button
															{...props}
															size="icon"
															variant="outline"
															aria-label={t('organizations.containers.more_actions')}>
															<MoreHorizontalIcon class="size-4" />
														</Button>
													{/snippet}
												</DropdownMenu.Trigger>
												<DropdownMenu.Content align="end">
													<DropdownMenu.Item onSelect={() => askContainerAction('pause')}
														>{t('organizations.containers.pause')}</DropdownMenu.Item>
													<DropdownMenu.Item onSelect={() => askContainerAction('unpause')}
														>{t('organizations.containers.resume')}</DropdownMenu.Item>
													<DropdownMenu.Item
														variant="destructive"
														onSelect={() => askContainerAction('kill')}
														>{t('organizations.containers.kill')}</DropdownMenu.Item>
													<DropdownMenu.Separator />
													<DropdownMenu.Item variant="destructive" onSelect={askDelete}
														>{t('organizations.containers.delete_container')}</DropdownMenu.Item>
												</DropdownMenu.Content>
											</DropdownMenu.Root>
										{/if}
									{:else if active === 'pods' && canManage}
										<Button
											size="sm"
											variant="outline"
											onclick={() => askPodAction('start')}
											disabled={actionBusy}
											><PlayIcon />{t('organizations.containers.start')}</Button>
										<Button
											size="sm"
											variant="outline"
											onclick={() => askPodAction('stop')}
											disabled={actionBusy}
											><SquareIcon />{t('organizations.containers.stop')}</Button>
										<Button
											size="sm"
											variant="outline"
											onclick={() => askPodAction('restart')}
											disabled={actionBusy}>
											<RotateCwIcon />
											{t('organizations.containers.restart')}
										</Button>

										<Button
											size="sm"
											variant="outline"
											onclick={() => askPodAction('kill')}
											disabled={actionBusy}>{t('organizations.containers.kill')}</Button>
									{:else if active === 'networks' && canManage}
										<Button
											size="sm"
											variant="outline"
											onclick={() => askNetworkMember('connect')}
											disabled={actionBusy}
											>{t('organizations.containers.connect_container')}</Button>
										<Button
											size="sm"
											variant="outline"
											onclick={() => askNetworkMember('disconnect')}
											disabled={actionBusy}>{t('organizations.containers.disconnect')}</Button>
									{:else if active === 'volumes'}
										<Button
											size="sm"
											variant="outline"
											onclick={() => (browsingVolume = selected?.id ?? null)}>
											<FolderOpenIcon />
											{t('organizations.containers.browse_files')}
										</Button>
									{/if}
									{#if canManage}
										<Button
											size="sm"
											variant="destructive"
											onclick={askDelete}
											disabled={actionBusy}
											><Trash2Icon />{t('organizations.common.delete')}</Button>
									{/if}
								</div>

								{#if detailRows.length}
									<div class="lp-inset overflow-hidden rounded-xl bg-muted/40">
										<dl class="divide-y text-sm">
											{#each detailRows as [label, value] (label)}
												<div class="grid grid-cols-[7rem_minmax(0,1fr)] gap-3 px-4 py-2">
													<dt class="text-muted-foreground">{label}</dt>
													<dd class="min-w-0 break-words font-medium">{value}</dd>
												</div>
											{/each}
										</dl>
									</div>
								{/if}
								<div class="mt-3">
									<Button
										size="sm"
										variant="ghost"
										class="text-muted-foreground"
										onclick={() => (showDetailJson = !showDetailJson)}>
										{showDetailJson
											? t('organizations.containers.hide_raw_json')
											: t('organizations.containers.show_raw_json')}
									</Button>
									{#if showDetailJson}
										<div class="mt-2">
											<DebugJsonPanel
												label={t('organizations.containers.sanitized_inspect', {
													resource: active.slice(0, -1),
												})}
												value={selected.data} />
										</div>
									{/if}
								</div>
							{:else}
								<div class="rounded-2xl border border-dashed border-foreground/15 p-8 text-center text-sm text-muted-foreground">
									{t('organizations.containers.select_resource')}
								</div>
							{/if}
						</div>
					</div>
				{:else if active === 'monitoring'}
					<div class="space-y-6">
						<div class="grid min-w-0 grid-cols-1 gap-4 md:grid-cols-2 xl:grid-cols-3">
							{#each monitorTiles as tile (tile.key)}
								<ResourceGauge label={tile.label} percent={tile.percent} detail={tile.detail} />
							{/each}
						</div>
						<div class="lp-raised rounded-2xl border bg-card">
							<div class="p-4 sm:p-5">
								<div class="mb-4 flex flex-wrap items-center justify-between gap-3">
									<div>
										<h3 class="font-semibold tracking-tight">
											{t('organizations.containers.top_consumers')}
										</h3>
										<p class="mt-1 text-sm text-muted-foreground">
											{t('organizations.containers.live_cpu_memory')}
										</p>
									</div>
									<Select.Root
										type="single"
										value={usageSort}
										onValueChange={(value) => (usageSort = value as UsageSort)}>
										<Select.Trigger
											class="w-40"
											aria-label={t('organizations.containers.sort_usage')}>
											<ArrowDownUpIcon class="mr-2 size-4" />
											{usageSort === 'cpu'
												? t('organizations.containers.cpu_usage')
												: usageSort === 'memory'
													? t('organizations.containers.memory_usage')
													: t('organizations.containers.disk_usage')}
										</Select.Trigger>
										<Select.Content>
											<Select.Item value="cpu" label={t('organizations.containers.cpu_usage')}
												>{t('organizations.containers.cpu_usage')}</Select.Item>
											<Select.Item value="memory" label={t('organizations.containers.memory_usage')}
												>{t('organizations.containers.memory_usage')}</Select.Item>
											<Select.Item value="disk" label={t('organizations.containers.disk_usage')}
												>{t('organizations.containers.disk_usage')}</Select.Item>
										</Select.Content>
									</Select.Root>
								</div>
								{#if monitorUsage.length === 0}
									<div
										class="rounded-2xl border border-dashed p-8 text-center text-sm text-muted-foreground">
										{t('organizations.containers.no_running_containers')}
									</div>
								{:else}
									<div class="lp-inset overflow-hidden rounded-xl">
										{#each monitorUsage as item (item.id)}
											<button
												type="button"
												class="grid w-full grid-cols-[minmax(0,1fr)_auto] gap-x-4 gap-y-1 border-b px-3 py-3 text-left transition-colors duration-300 ease-[var(--lp-ease)] last:border-b-0 hover:bg-muted/60 active:scale-[0.995] sm:grid-cols-[minmax(0,1fr)_7rem_9rem_8rem] sm:items-center"
												onclick={() => void openContainerFromMonitoring(item.id)}>
												<span class="min-w-0 truncate font-medium">{item.name}</span>
												<span
													class="font-mono text-xs tabular-nums text-muted-foreground sm:text-right">
													{t('organizations.containers.cpu_label')}
													{item.cpu === null
														? t('organizations.containers.unavailable')
														: `${item.cpu.toFixed(1)}%`}
												</span>
												<span
													class="font-mono text-xs tabular-nums text-muted-foreground sm:text-right">
													{t('organizations.containers.ram_label')}
													{item.memory === null
														? t('organizations.containers.unavailable')
														: formatBytes(item.memory)}
												</span>
												<span
													class="font-mono text-xs tabular-nums text-muted-foreground sm:text-right">
													{t('organizations.containers.disk_label')}
													{item.disk === null
														? t('organizations.containers.unavailable')
														: formatBytes(item.disk)}
												</span>
											</button>
										{/each}
									</div>
								{/if}
							</div>
						</div>
					</div>
				{:else if active === 'events'}
					{#if events.length === 0}
						<div class="rounded-2xl border border-dashed border-foreground/15 p-8 text-center text-sm text-muted-foreground">
							{t('organizations.containers.no_recent_events')}
						</div>
					{:else}
						<div class="lp-inset overflow-hidden rounded-xl bg-muted/40 font-mono text-xs">
							{#each events as event, i (i)}
								<div class="border-b px-3 py-1.5 last:border-b-0">
									<span class="text-muted-foreground"
										>{String(event.Type ?? event.type ?? '')}</span>
									<span class="mx-2"
										>{String(
											event.Action ?? event.action ?? event.Status ?? event.status ?? '',
										)}</span>
									<span
										>{String(
											event.Name ?? event.name ?? (event.Actor as PodmanRecord)?.Name ?? '',
										)}</span>
								</div>
							{/each}
						</div>
					{/if}
				{:else if active === 'disk-usage'}
					<div class="flex flex-wrap items-center justify-end gap-2">
						<Button size="sm" variant="outline" onclick={() => (showRaw = !showRaw)}>
							{showRaw
								? t('organizations.containers.hide_raw')
								: t('organizations.containers.show_raw')}
						</Button>

						{#if canManage}
							<Button size="sm" variant="outline" onclick={askPruneBuildCache} disabled={actionBusy}
								>{t('organizations.containers.prune_build_cache')}</Button>
						{/if}
					</div>
					{#if dfCards.length === 0}
						<DebugJsonPanel label="podman system df" value={diskUsage} />
					{:else}
						<div class="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
							{#each dfCards as card (card.title)}
								<Card.Root size="sm">
									<Card.Header><Card.Title class="text-base">{card.title}</Card.Title></Card.Header>
									<Card.Content class="space-y-1 text-sm">
										{#each card.rows as [label, value] (label)}
											<div class="flex justify-between gap-3">
												<span class="text-muted-foreground">{label}</span>
												<span class="font-medium">{value}</span>
											</div>
										{/each}
									</Card.Content>
								</Card.Root>
							{/each}
						</div>
						{#if showRaw}<DebugJsonPanel label="podman system df" value={diskUsage} />{/if}
					{/if}
				{:else if active === 'health'}
					<div class="flex justify-end">
						<Button size="sm" variant="outline" onclick={() => (showRaw = !showRaw)}>
							{showRaw
								? t('organizations.containers.hide_raw')
								: t('organizations.containers.show_raw')}
						</Button>
					</div>
					{#if healthCards.length === 0}
						<DebugJsonPanel label="podman info / version" value={overview} />
					{:else}
						<div class="grid gap-4 sm:grid-cols-2">
							{#each healthCards as card (card.title)}
								<Card.Root size="sm">
									<Card.Header><Card.Title class="text-base">{card.title}</Card.Title></Card.Header>
									<Card.Content class="space-y-1 text-sm">
										{#each card.rows as [label, value] (label)}
											<div class="flex justify-between gap-3">
												<span class="text-muted-foreground">{label}</span>
												<span class="max-w-[60%] truncate text-right font-medium" title={value}
													>{value}</span>
											</div>
										{/each}
									</Card.Content>
								</Card.Root>
							{/each}
						</div>
						{#if showRaw}<DebugJsonPanel label="podman info / version" value={overview} />{/if}
					{/if}
				{/if}
			</Card.Content>
		</Card.Root>
	{/if}
</div>

<Dialog.Root
	open={termView !== null}
	onOpenChange={(open) => {
		if (!open) termView = null;
	}}>
	<Dialog.Content
		showCloseButton={false}
		class="flex h-[100dvh] w-screen max-w-[100vw] min-w-0 flex-col gap-0 overflow-hidden rounded-none p-0 sm:h-[80vh] sm:w-[80vw] sm:max-w-[80vw] sm:rounded-2xl">
		<Dialog.Header
			class="order-last flex-row items-center justify-between gap-3 border-t px-4 py-3 sm:order-first sm:border-t-0 sm:border-b">
			<Dialog.Title class="font-mono text-xs sm:text-sm">
				{#if termView?.mode === 'logs'}logs{:else if termView?.mode === 'top'}top{:else}sh{/if}
				· {termView?.name}
			</Dialog.Title>
			<div class="flex items-center gap-2">
				{#if termView?.mode === 'top'}
					<span class="text-success flex items-center gap-1 text-xs">
						<span class="size-1.5 animate-pulse rounded-full bg-current"></span>live · 2s
					</span>
				{/if}
				<Dialog.Close
					class="bg-secondary hover:bg-secondary/80 inline-flex size-7 shrink-0 items-center justify-center rounded-md">
					<XIcon class="size-4" />
					<span class="sr-only">{t('organizations.common.close')}</span>
				</Dialog.Close>
			</div>
		</Dialog.Header>

		{#if termView?.mode === 'shell'}
			{#key termView.id}
				<ContainerShell {organizationId} {serverId} containerId={termView.id} />
			{/key}
		{:else if termView?.mode === 'logs'}
			{#key termView.id}
				<ContainerLogView {organizationId} {serverId} containerId={termView.id} />
			{/key}
		{:else if termView}
			<div
				bind:this={termBody}
				class="min-h-0 flex-1 overflow-auto bg-white p-3 font-mono text-[11px] leading-snug whitespace-pre sm:p-4 sm:text-[13px] sm:leading-relaxed dark:bg-[#1e1e1e]">
				{termView.lines.join('\n')}
			</div>
		{/if}
	</Dialog.Content>
</Dialog.Root>

<Dialog.Root
	open={browsingVolume !== null}
	onOpenChange={(open) => {
		if (!open) browsingVolume = null;
	}}>
	<Dialog.Content
		showCloseButton={false}
		class="flex h-[100dvh] w-screen max-w-[100vw] min-w-0 flex-col gap-0 overflow-hidden rounded-none p-0 sm:h-[95vh] sm:w-[95vw] sm:max-w-[95vw] sm:gap-6 sm:rounded-2xl sm:p-6">
		<Dialog.Header
			class="order-last flex-row items-center justify-between gap-3 border-t px-4 py-3 sm:order-first sm:border-none sm:p-0">
			<Dialog.Title class="font-mono text-xs sm:text-sm">volume · {browsingVolume}</Dialog.Title>
			<Dialog.Close
				class="bg-secondary hover:bg-secondary/80 inline-flex size-7 shrink-0 items-center justify-center rounded-md sm:absolute sm:top-4 sm:right-4">
				<XIcon class="size-4" />
				<span class="sr-only">{t('organizations.common.close')}</span>
			</Dialog.Close>
		</Dialog.Header>
		{#if browsingVolume}
			{#key browsingVolume}
				<div class="min-h-0 flex-1 overflow-hidden">
					<VolumeFileWorkspace
						title={t('organizations.containers.volume_title', { name: browsingVolume })}
						client={volumeWorkspace(organizationId, serverId, browsingVolume)}
						canWrite={canManage} />
				</div>
			{/key}
		{/if}
	</Dialog.Content>
</Dialog.Root>

<AlertDialog.Root
	open={pending !== null}
	onOpenChange={(open) => {
		if (!open && !actionBusy) {
			pending = null;
		}
	}}>
	<AlertDialog.Content class="w-[calc(100vw-2rem)] max-w-lg min-w-0">
		<AlertDialog.Header>
			<AlertDialog.Title>{pending?.label}</AlertDialog.Title>
			<AlertDialog.Description>
				{pending?.kind === 'delete'
					? t('organizations.containers.cannot_undo')
					: t('organizations.containers.affects_live')}
			</AlertDialog.Description>
		</AlertDialog.Header>
		<AlertDialog.Footer class="flex-wrap gap-2">
			<AlertDialog.Cancel disabled={actionBusy}
				>{t('organizations.common.cancel')}</AlertDialog.Cancel>
			<DeleteConfirmControl
				label={pending?.label ?? t('organizations.common.confirm')}
				pendingLabel={t('organizations.common.working')}
				variant={pending?.kind === 'delete' ? 'destructive' : 'primary'}
				disabled={!pending || actionBusy}
				onconfirm={confirmPending} />
		</AlertDialog.Footer>
	</AlertDialog.Content>
</AlertDialog.Root>
