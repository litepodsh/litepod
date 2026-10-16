<script lang="ts">
	import { onMount } from 'svelte';
	import { page } from '$app/state';
	import { goto } from '$app/navigation';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Select from '#lib/components/ui/select/index.js';
	import * as Tabs from '#lib/components/ui/tabs/index.js';
	import RefreshCwIcon from '@lucide/svelte/icons/refresh-cw';
	import ServerMap from '#lib/components/servers/server-map.svelte';
	import ServerGlobe from '#lib/components/servers/server-globe.svelte';
	import {
		listServers,
		listServerApplications,
		listServerPorts,
		listServerGeo,
		type Server,
		type ServerAssignedApplication,
		type ServerGeo,
		type ServerPort,
	} from '#lib/clients/server-client.js';
	import { getSystemVersion } from '#lib/clients/system-client.js';
	import { serverGroups } from '#lib/utils/server-visibility.js';
	import { t } from '#lib/i18n/index.svelte.js';

	const organizationId = page.params.id!;

	let servers = $state<Server[]>([]);
	let isCloud = $state(false);
	let serverId = $state('');
	let apps = $state<ServerAssignedApplication[]>([]);
	let ports = $state<ServerPort[]>([]);
	let loading = $state(false);
	let error = $state<string | null>(null);

	let tab = $state<'network' | 'world'>('world');
	let geo = $state<ServerGeo[]>([]);
	let geoLoading = $state(false);
	let geoError = $state<string | null>(null);
	let geoLoaded = false;

	let serverOptions = $derived.by(() => {
		const groups = serverGroups(servers, isCloud);
		return [...groups.local, ...groups.remote].filter((server) => server.state === 'ready');
	});
	let activeServer = $derived(serverOptions.find((server) => server.id === serverId) ?? null);

	async function load() {
		if (!serverId) return;
		loading = true;
		error = null;
		try {
			const [assigned, serverPorts] = await Promise.all([
				listServerApplications(organizationId, serverId),
				listServerPorts(organizationId, serverId).catch(() => [] as ServerPort[]),
			]);
			apps = assigned;
			ports = serverPorts;
		} catch (caught) {
			error =
				caught instanceof Error ? caught.message : t('organizations.topology.apps_load_failed');
		} finally {
			loading = false;
		}
	}

	/** Geolocation calls out to ip.guide; loaded on mount since the world tab is the default. */
	async function loadGeo() {
		geoLoading = true;
		geoError = null;
		try {
			geo = await listServerGeo(organizationId);
			geoLoaded = true;
		} catch (caught) {
			geoError =
				caught instanceof Error ? caught.message : t('organizations.topology.globe_load_failed');
		} finally {
			geoLoading = false;
		}
	}

	function changeTab(next: string) {
		tab = next === 'world' ? 'world' : 'network';
		if (tab === 'world' && !geoLoaded && !geoLoading) void loadGeo();
	}

	function changeServer(id: string) {
		serverId = id;
		const url = new URL(page.url.href);
		url.searchParams.set('server', id);
		void goto(`${url.pathname}${url.search}`, {
			replace: true,
			reset: false,
		});
		void load();
	}

	onMount(async () => {
		void loadGeo();
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
			if (serverId) await load();
		} catch (caught) {
			error =
				caught instanceof Error ? caught.message : t('organizations.common.servers_load_failed');
		}
	});
</script>

<div class="flex flex-col gap-5 px-4 py-6 sm:px-6">
	<div class="flex flex-wrap items-start justify-between gap-4">
		<div>
			<h1 class="text-xl font-semibold tracking-tight">{t('organizations.topology.title')}</h1>
			<p class="text-sm text-muted-foreground">
				{t('organizations.topology.description')}
			</p>
		</div>
		<div class="flex w-full flex-wrap items-center gap-2 sm:w-auto" hidden={tab !== 'network'}>
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
			<Button variant="outline" onclick={() => void load()} disabled={loading || !serverId}>
				<RefreshCwIcon class={loading ? 'animate-spin' : ''} />{t('organizations.common.refresh')}
			</Button>
		</div>
	</div>

	<Tabs.Root value={tab} onValueChange={changeTab}>
		<Tabs.List>
			<Tabs.Trigger value="world">{t('organizations.topology.tab_world')}</Tabs.Trigger>
			<Tabs.Trigger value="network">{t('organizations.topology.tab_network')}</Tabs.Trigger>
		</Tabs.List>
		<Tabs.Content value="network" class="mt-4">
			{#if serverOptions.length === 0}
				<p class="rounded-lg border border-dashed p-6 text-sm text-muted-foreground">
					{t('organizations.common.no_ready_servers')}
				</p>
			{:else}
				<ServerMap {apps} {ports} {loading} {error} onRefresh={load} />
			{/if}
		</Tabs.Content>
		<Tabs.Content value="world" class="mt-4">
			<ServerGlobe
				servers={geo}
				{isCloud}
				loading={geoLoading}
				error={geoError}
				onRefresh={loadGeo} />
		</Tabs.Content>
	</Tabs.Root>
</div>
