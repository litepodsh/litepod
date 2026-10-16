<script lang="ts">
	import { Handle, Position } from '@xyflow/svelte';
	import NetworkIcon from '@lucide/svelte/icons/network';
	import type { NetworkNodeData } from '#lib/utils/graph-layout.js';

	let { data }: { data: NetworkNodeData } = $props();
</script>

<div class="node" class:shared={data.scope === 'shared'}>
	<Handle type="target" position={Position.Left} />
	<div class="head">
		<NetworkIcon size={13} strokeWidth={2} />
		<span class="kind">network</span>
		<span class="pill" class:shared={data.scope === 'shared'}>{data.scope}</span>
	</div>
	<p class="title" title={data.label}>{data.label}</p>
	<dl class="body">
		<div>
			<dt>driver</dt>
			<dd>{data.driver ?? '—'}</dd>
		</div>
		<div>
			<dt>app members</dt>
			<dd>{data.appCount}</dd>
		</div>
		<div>
			<dt>outside</dt>
			<dd>{data.extCount}</dd>
		</div>
	</dl>
</div>

<style>
	.node {
		width: 210px;
		border: 1px solid var(--border);
		border-radius: 12px;
		background: var(--muted);
		color: var(--card-foreground);
		box-shadow: 0 8px 24px -14px rgb(0 0 0 / 0.45);
		font-family: var(--font-sans);
	}
	.node.shared {
		border-color: color-mix(in oklch, var(--warning) 55%, transparent);
	}
	.head {
		display: flex;
		align-items: center;
		gap: 7px;
		padding: 8px 10px;
		border-bottom: 1px solid var(--border);
		color: var(--muted-foreground);
	}
	.kind {
		font-family: var(--font-mono);
		font-size: 9.5px;
		font-weight: 600;
		letter-spacing: 0.11em;
		text-transform: uppercase;
	}
	.pill {
		margin-left: auto;
		font-family: var(--font-mono);
		font-size: 9px;
		font-weight: 600;
		letter-spacing: 0.08em;
		text-transform: uppercase;
		padding: 3px 6px;
		border-radius: 6px;
		background: color-mix(in oklch, var(--success) 18%, transparent);
		color: var(--success);
	}
	.pill.shared {
		background: color-mix(in oklch, var(--warning) 18%, transparent);
		color: var(--warning);
	}
	.title {
		margin: 0;
		padding: 8px 10px 2px;
		font-size: 12.5px;
		font-weight: 600;
		letter-spacing: -0.01em;
		word-break: break-word;
	}
	.body {
		margin: 0;
		padding: 0 10px 9px;
		display: flex;
		flex-direction: column;
		gap: 3px;
	}
	.body div {
		display: flex;
		justify-content: space-between;
		gap: 10px;
		font-family: var(--font-mono);
		font-size: 10.5px;
	}
	.body dt {
		color: var(--muted-foreground);
	}
	.body dd {
		margin: 0;
		color: var(--foreground);
		text-align: right;
	}
</style>
