<script lang="ts">
	import { Handle, Position } from '@xyflow/svelte';
	import ApplicationIcon from '#lib/components/projects/application-icon.svelte';
	import type { AppClusterNodeData } from '#lib/utils/server-graph.js';
	import { t } from '#lib/i18n/index.svelte.js';

	let { data }: { data: AppClusterNodeData } = $props();

	let failed = $state(new Set<string>());
</script>

<a class="node" href={data.href} title={t('organizations.topology.open_map', { name: data.name })}>
	<div class="head">
		<span class="icon">
			<ApplicationIcon
				templateKind={data.templateKind}
				catalogIconUrl={data.catalogIconUrl}
				iconUrl={data.iconUrl}
				applicationId={data.id}
				bind:failedApplicationIds={failed}
				class="size-4" />
		</span>
		<span class="title">{data.name}</span>
	</div>
	<p class="sub">{data.project} / {data.env}</p>
	{#if data.ports.length}
		<p class="ports" class:conflict={data.conflict}>{data.ports.join('  ')}</p>
	{/if}
	<div class="tags">
		<span class="tag">{data.deployType}</span>
		{#if data.ingress}<span class="tag ingress">{t('organizations.topology.ingress')}</span>{/if}
		{#if data.conflict}<span class="tag danger"
				>{t('organizations.topology.port_conflict', {
					ports: data.conflictPorts.join(', :'),
				})}</span
			>{/if}
	</div>
	<Handle type="source" position={Position.Bottom} id="down" />
</a>

<style>
	.node {
		display: block;
		width: 190px;
		border: 1px solid var(--border);
		border-radius: 12px;
		background: var(--card);
		color: var(--card-foreground);
		box-shadow: 0 8px 24px -14px rgb(0 0 0 / 0.45);
		font-family: var(--font-sans);
		text-decoration: none;
		transition:
			border-color 0.16s,
			transform 0.16s;
	}
	.node:hover {
		border-color: var(--primary);
		transform: translateY(-2px);
	}
	.node:focus-visible {
		outline: 2px solid var(--primary);
		outline-offset: 2px;
	}
	.head {
		display: flex;
		align-items: center;
		gap: 8px;
		padding: 9px 10px 4px;
	}
	.icon {
		flex: none;
		display: flex;
		align-items: center;
		justify-content: center;
		width: 22px;
		height: 22px;
		border-radius: 6px;
		background: var(--muted);
	}
	.title {
		font-size: 12.5px;
		font-weight: 600;
		letter-spacing: -0.01em;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.sub {
		margin: 0;
		padding: 0 10px;
		font-family: var(--font-mono);
		font-size: 10px;
		color: var(--muted-foreground);
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.ports {
		margin: 0;
		padding: 2px 10px 0;
		font-family: var(--font-mono);
		font-size: 9.5px;
		color: var(--muted-foreground);
		white-space: nowrap;
		overflow: hidden;
		text-overflow: ellipsis;
	}
	.ports.conflict {
		color: var(--destructive);
	}
	.tags {
		display: flex;
		flex-wrap: wrap;
		gap: 4px;
		padding: 7px 10px 9px;
	}
	.tag {
		font-family: var(--font-mono);
		font-size: 8.5px;
		font-weight: 600;
		letter-spacing: 0.06em;
		text-transform: uppercase;
		padding: 3px 6px;
		border-radius: 5px;
		background: var(--muted);
		color: var(--muted-foreground);
	}
	.tag.ingress {
		background: color-mix(in oklch, var(--success) 18%, transparent);
		color: var(--success);
	}
	.tag.danger {
		background: color-mix(in oklch, var(--destructive) 18%, transparent);
		color: var(--destructive);
	}
</style>
