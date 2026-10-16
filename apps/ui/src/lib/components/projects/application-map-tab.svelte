<script lang="ts">
	import SkeletonRows from '#lib/components/custom/skeleton-rows.svelte';
	import {
		SvelteFlow,
		Background,
		BackgroundVariant,
		Controls,
		MiniMap,
		Panel,
	} from '@xyflow/svelte';
	import '@xyflow/svelte/dist/style.css';
	import type { Edge, Node } from '@xyflow/svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Card from '#lib/components/ui/card/index.js';
	import RefreshCwIcon from '@lucide/svelte/icons/refresh-cw';
	import PlugZapIcon from '@lucide/svelte/icons/plug-zap';
	import type {
		ApplicationNetworkOverviewDto,
		ContainerNetStatDto,
		NetworkProbePairDto,
	} from '#lib/clients/project-client.js';
	import { networkMember } from '#lib/clients/podman-console-client.js';
	import { toast } from '#lib/utils/toast.js';
	import { t } from '#lib/i18n/index.svelte.js';
	import { buildTopologyGraph, formatRate } from '#lib/utils/graph-layout.js';
	import ContainerNode from './application-map-tab/container-node.svelte';
	import NetworkNode from './application-map-tab/network-node.svelte';
	import PacketEdge from './application-map-tab/packet-edge.svelte';

	let {
		overview,
		statuses = {},
		organizationId,
		serverId = null,
		loading = false,
		error = null,
		onRefresh,
		onProbe,
		onStats,
	}: {
		overview: ApplicationNetworkOverviewDto | null;
		statuses?: Record<string, string>;
		organizationId: string;
		serverId?: string | null;
		loading?: boolean;
		error?: string | null;
		onRefresh: () => Promise<void>;
		/** Runs the active reachability probe and returns its pairs. */
		onProbe?: () => Promise<NetworkProbePairDto[]>;
		/** Fetches cumulative per-container network counters for the throughput view. */
		onStats?: () => Promise<ContainerNetStatDto[]>;
	} = $props();

	let detaching = $state<string | null>(null);
	let probes = $state.raw<NetworkProbePairDto[]>([]);
	let probing = $state(false);

	// bytes/s per container. `podman stats --no-stream` already reports NetIO as a
	// short-window byte delta (roughly per second), not a monotonic total, so we
	// use each sample directly — no diffing. A container keeps its last value
	// until the next sample, and idle reads "0 B/s"; it never blanks out.
	let rates = $state.raw<Record<string, number>>({});

	$effect(() => {
		if (!onStats || !overview || overview.containers.length === 0) return;
		let cancelled = false;
		const owned = overview.containers.map((c) => c.name);
		async function poll() {
			try {
				const rows = await onStats!();
				if (cancelled) return;
				const sample: Record<string, number> = {};
				for (const name of owned) sample[name] = 0;
				for (const row of rows) sample[row.container] = row.net_rx_bytes + row.net_tx_bytes;
				rates = sample;
			} catch {
				// transient; keep the last rates
			}
		}
		void poll();
		const timer = setInterval(poll, 3000);
		return () => {
			cancelled = true;
			clearInterval(timer);
		};
	});

	// A refresh changes the topology, so any probe result is now stale.
	async function refresh() {
		probes = [];
		await onRefresh();
	}

	async function runProbe() {
		if (!onProbe) return;
		probing = true;
		try {
			probes = await onProbe();
			if (probes.length === 0) toast.info(t('projects.application_map.nothing_to_probe'));
		} catch (caught) {
			toast.error(
				caught instanceof Error ? caught.message : t('projects.application_map.probe_failed'),
			);
		} finally {
			probing = false;
		}
	}

	async function detach(container: string, network: string) {
		if (!serverId) return;
		detaching = container;
		try {
			await networkMember(organizationId, serverId, network, 'disconnect', container);
			toast.success(t('projects.application_map.detached_toast', { container, network }));
			await refresh();
		} catch (caught) {
			const message =
				caught instanceof Error ? caught.message : t('projects.application_map.could_not_detach');
			toast.error(
				/403|forbidden|admin/i.test(message)
					? t('projects.application_map.detach_admin_required')
					: message,
			);
		} finally {
			detaching = null;
		}
	}

	const nodeTypes = { containerNode: ContainerNode, networkNode: NetworkNode };
	const edgeTypes = { packet: PacketEdge };

	let nodes = $state.raw<Node[]>([]);
	// Deep-proxied so live rate updates can be patched onto existing edges
	// without rebuilding the array (which would remount every edge and flash).
	let edges = $state<Edge[]>([]);

	// Structural build: only on topology / status / probe changes, not on rate ticks.
	$effect(() => {
		const graph = buildTopologyGraph(overview, statuses, probes);
		nodes = graph.nodes;
		edges = graph.edges;
	});

	// Patch bytes/s onto the attachment edges in place: the number animates to
	// its new value and stays; it never disappears.
	$effect(() => {
		for (const edge of edges) {
			const data = edge.data as { kind?: string; container?: string; rate?: number } | undefined;
			if (data?.kind !== 'attach' || !data.container) continue;
			const rate = rates[data.container];
			if (rate === undefined) continue;
			data.rate = rate;
			edge.label = formatRate(rate);
		}
	});

	// Follow the app theme (mode-watcher toggles `.dark` on <html>). Read it from
	// the DOM rather than importing mode-watcher, which is awkward under jsdom.
	let colorMode = $state<'light' | 'dark'>('light');
	$effect(() => {
		const root = document.documentElement;
		const sync = () => (colorMode = root.classList.contains('dark') ? 'dark' : 'light');
		sync();
		const observer = new MutationObserver(sync);
		observer.observe(root, { attributes: true, attributeFilter: ['class'] });
		return () => observer.disconnect();
	});

	const isEmpty = $derived(
		!overview || (overview.networks.length === 0 && overview.containers.length === 0),
	);
</script>

<Card.Root>
	<Card.Header>
		<div class="flex flex-wrap items-center justify-between gap-3">
			<div>
				<Card.Title>{t('projects.application_map.title')}</Card.Title>
				<Card.Description>
					{t('projects.application_map.description')}
				</Card.Description>
			</div>
			<div class="flex items-center gap-2">
				{#if onProbe}
					<Button variant="outline" disabled={probing || loading} onclick={() => void runProbe()}>
						<PlugZapIcon class={probing ? 'animate-pulse' : ''} />
						{probing
							? t('projects.application_map.probing')
							: t('projects.application_map.test_reachability')}
					</Button>
				{/if}
				<Button variant="outline" disabled={loading} onclick={() => void refresh()}>
					<RefreshCwIcon class={loading ? 'animate-spin' : ''} />{t(
						'projects.application_general.refresh',
					)}
				</Button>
			</div>
		</div>
	</Card.Header>
	<Card.Content>
		{#if error}
			<div
				class="rounded-lg border border-destructive/40 bg-destructive/10 p-4 text-sm text-destructive">
				{error}
			</div>
		{:else if loading && !overview}
			<SkeletonRows rows={1} height="h-64" label={t('projects.application_map.loading_topology')} />
		{:else if isEmpty}
			<p class="rounded-lg border border-dashed p-6 text-sm text-muted-foreground">
				{t('projects.dialogs_common.no_deployed_containers')}
			</p>
		{:else}
			<div class="topology h-[600px] overflow-hidden rounded-lg border">
				<SvelteFlow
					bind:nodes
					bind:edges
					{nodeTypes}
					{edgeTypes}
					{colorMode}
					fitView
					fitViewOptions={{ padding: 0.3 }}
					minZoom={0.3}
					nodesConnectable={false}
					elementsSelectable={false}
					proOptions={{ hideAttribution: true }}>
					<Background variant={BackgroundVariant.Dots} gap={26} size={1} />
					<Controls showLock={false} />
					<MiniMap pannable zoomable nodeColor="var(--muted-foreground)" />
					<Panel position="bottom-center">
						<div class="legend">
							<span><i class="ln attach"></i> {t('projects.application_map.legend_attach')}</span>
							<span><i class="ln peer"></i> {t('projects.application_map.legend_peer')}</span>
						</div>
					</Panel>
					{#if overview && overview.findings.length}
						<Panel position="top-right">
							<div class="findings">
								{#each overview.findings as finding (finding.code + ':' + (finding.container ?? ''))}
									<div class="finding" class:warn={finding.severity === 'warning'}>
										<p class="code">{finding.code}</p>
										<p class="msg">{finding.message}</p>
										{#if finding.container && finding.network && serverId}
											<button
												class="detach"
												disabled={detaching === finding.container}
												onclick={() => detach(finding.container!, finding.network!)}>
												{detaching === finding.container
													? t('projects.application_map.detaching')
													: t('projects.application_map.detach_from', { network: finding.network })}
											</button>
										{/if}
									</div>
								{/each}
							</div>
						</Panel>
					{/if}
				</SvelteFlow>
			</div>
		{/if}
	</Card.Content>
</Card.Root>

<style>
	.topology {
		background: var(--muted);
	}
	.topology :global(.svelte-flow) {
		--xy-background-color: var(--muted);
		--xy-node-border-default: transparent;
		--xy-node-background-color-default: transparent;
		--xy-node-boxshadow-default: none;
	}
	/* Honor reduced-motion: kill the animated-edge dash march. */
	@media (prefers-reduced-motion: reduce) {
		.topology :global(.svelte-flow__edge.animated .svelte-flow__edge-path) {
			animation: none;
		}
	}

	.legend {
		display: flex;
		flex-direction: column;
		gap: 4px;
		max-width: 640px;
		padding: 7px 12px;
		border: 1px solid var(--border);
		border-radius: 10px;
		background: color-mix(in oklch, var(--card) 88%, transparent);
		backdrop-filter: blur(4px);
		font-size: 11px;
		line-height: 1.35;
		color: var(--muted-foreground);
	}
	.legend span {
		display: flex;
		align-items: center;
		gap: 8px;
	}
	.legend .ln {
		flex: none;
		width: 22px;
		height: 0;
		border-top: 2px solid var(--muted-foreground);
	}
	.legend .ln.attach {
		border-color: var(--muted-foreground);
	}
	.legend .ln.peer {
		border-color: var(--success);
	}

	.findings {
		display: flex;
		flex-direction: column;
		gap: 6px;
		max-width: 280px;
	}
	.finding {
		border: 1px solid var(--border);
		border-left: 3px solid var(--muted-foreground);
		border-radius: 8px;
		background: var(--card);
		padding: 8px 10px;
	}
	.finding.warn {
		border-left-color: var(--warning);
	}
	.finding .code {
		margin: 0;
		font-family: var(--font-mono);
		font-size: 10px;
		font-weight: 600;
		letter-spacing: 0.05em;
		text-transform: uppercase;
		color: var(--muted-foreground);
	}
	.finding.warn .code {
		color: var(--warning);
	}
	.finding .msg {
		margin: 4px 0 0;
		font-size: 12px;
		line-height: 1.45;
		color: var(--foreground);
	}
	.detach {
		margin-top: 8px;
		appearance: none;
		border: 1px solid var(--border);
		border-radius: 7px;
		background: var(--background);
		color: var(--foreground);
		font: 500 11px/1 var(--font-sans);
		padding: 6px 10px;
		cursor: pointer;
	}
	.detach:hover:not(:disabled) {
		border-color: var(--warning);
		color: var(--warning);
	}
	.detach:disabled {
		opacity: 0.6;
		cursor: default;
	}
</style>
