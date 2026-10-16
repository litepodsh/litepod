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
	import RefreshCwIcon from '@lucide/svelte/icons/refresh-cw';
	import type { ServerAssignedApplication, ServerPort } from '#lib/clients/server-client.js';
	import { buildServerGraph } from '#lib/utils/server-graph.js';
	import AppClusterNode from './server-map/app-cluster-node.svelte';
	import PacketEdge from '#lib/components/projects/application-map-tab/packet-edge.svelte';
	import { t } from '#lib/i18n/index.svelte.js';

	let {
		apps,
		ports = [],
		loading = false,
		error = null,
		onRefresh,
	}: {
		apps: ServerAssignedApplication[];
		ports?: ServerPort[];
		loading?: boolean;
		error?: string | null;
		onRefresh: () => Promise<void>;
	} = $props();

	const nodeTypes = { appCluster: AppClusterNode };
	const edgeTypes = { packet: PacketEdge };

	let nodes = $state.raw<Node[]>([]);
	let edges = $state.raw<Edge[]>([]);
	$effect(() => {
		const graph = buildServerGraph(apps, ports);
		nodes = graph.nodes;
		edges = graph.edges;
	});

	let colorMode = $state<'light' | 'dark'>('light');
	$effect(() => {
		const root = document.documentElement;
		const sync = () => (colorMode = root.classList.contains('dark') ? 'dark' : 'light');
		sync();
		const observer = new MutationObserver(sync);
		observer.observe(root, { attributes: true, attributeFilter: ['class'] });
		return () => observer.disconnect();
	});
</script>

{#if error}
	<div
		class="rounded-lg border border-destructive/40 bg-destructive/10 p-4 text-sm text-destructive">
		{error}
	</div>
{:else if loading && apps.length === 0}
	<SkeletonRows rows={1} height="h-64" label={t('organizations.topology.loading')} />
{:else if apps.length === 0}
	<p class="rounded-lg border border-dashed p-6 text-sm text-muted-foreground">
		{t('organizations.topology.empty')}
	</p>
{:else}
	<div class="topology h-[70vh] min-h-[440px] overflow-hidden rounded-lg border">
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
			<Panel position="top-right">
				<Button variant="outline" size="sm" disabled={loading} onclick={() => void onRefresh()}>
					<RefreshCwIcon class={loading ? 'animate-spin' : ''} />{t('organizations.common.refresh')}
				</Button>
			</Panel>
			<Panel position="bottom-center">
				<div class="legend">
					{t('organizations.topology.legend')}
				</div>
			</Panel>
		</SvelteFlow>
	</div>
{/if}

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
	@media (prefers-reduced-motion: reduce) {
		.topology :global(.svelte-flow__edge.animated .svelte-flow__edge-path) {
			animation: none;
		}
	}
	.legend {
		max-width: 560px;
		padding: 7px 12px;
		border: 1px solid var(--border);
		border-radius: 10px;
		background: color-mix(in oklch, var(--card) 88%, transparent);
		backdrop-filter: blur(4px);
		font-size: 11px;
		line-height: 1.4;
		color: var(--muted-foreground);
	}
</style>
