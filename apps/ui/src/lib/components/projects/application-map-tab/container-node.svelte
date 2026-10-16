<script lang="ts">
	import { Handle, Position } from '@xyflow/svelte';
	import BoxIcon from '@lucide/svelte/icons/box';
	import type { ContainerNodeData } from '#lib/utils/graph-layout.js';

	let { data }: { data: ContainerNodeData } = $props();

	const LED_LABEL: Record<ContainerNodeData['led'], string> = {
		running: 'Running',
		degraded: 'Degraded',
		error: 'Error',
		stopped: 'Stopped',
		unknown: 'Status unknown',
	};
</script>

<div class="node">
	<div class="head">
		<BoxIcon size={13} strokeWidth={2} />
		<span class="kind">container</span>
		<span class="led {data.led}" title={LED_LABEL[data.led]} aria-label={LED_LABEL[data.led]}
		></span>
	</div>
	<p class="title" title={data.label}>{data.label}</p>
	<dl class="body">
		<div>
			<dt>service</dt>
			<dd>{data.service ?? '—'}</dd>
		</div>
		{#if data.ips.length}
			<div>
				<dt>ip</dt>
				<dd>{data.ips.join(', ')}</dd>
			</div>
		{/if}
		{#if data.aliases.length}
			<div>
				<dt>alias</dt>
				<dd>{data.aliases.join(', ')}</dd>
			</div>
		{/if}
	</dl>
	<Handle type="source" position={Position.Right} id="net" />
	<Handle type="target" position={Position.Top} id="peer-in" />
	<Handle type="source" position={Position.Bottom} id="peer-out" />
</div>

<style>
	.node {
		width: 190px;
		border: 1px solid var(--border);
		border-radius: 12px;
		background: var(--card);
		color: var(--card-foreground);
		box-shadow: 0 8px 24px -14px rgb(0 0 0 / 0.45);
		font-family: var(--font-sans);
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
	.led {
		margin-left: auto;
		width: 8px;
		height: 8px;
		border-radius: 50%;
		background: var(--muted-foreground);
	}
	.led.running {
		background: var(--success);
		animation: pulse 1.6s ease-out infinite;
		--pulse: color-mix(in oklch, var(--success) 40%, transparent);
	}
	.led.degraded {
		background: var(--warning);
		animation: pulse 1.6s ease-out infinite;
		--pulse: color-mix(in oklch, var(--warning) 40%, transparent);
	}
	.led.error {
		background: var(--destructive);
		animation: pulse 1s ease-out infinite;
		--pulse: color-mix(in oklch, var(--destructive) 40%, transparent);
	}
	.led.stopped,
	.led.unknown {
		background: var(--muted-foreground);
	}
	@keyframes pulse {
		0% {
			box-shadow: 0 0 0 0 var(--pulse);
		}
		70% {
			box-shadow: 0 0 0 6px transparent;
		}
		100% {
			box-shadow: 0 0 0 0 transparent;
		}
	}
	@media (prefers-reduced-motion: reduce) {
		.led {
			animation: none !important;
		}
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
		overflow-wrap: anywhere;
	}
</style>
