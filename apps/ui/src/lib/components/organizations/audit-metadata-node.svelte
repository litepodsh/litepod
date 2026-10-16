<script lang="ts">
	type Props = {
		value: unknown;
		name?: string;
		depth?: number;
	};

	let { value, name, depth = 0 }: Props = $props();

	const isObject = $derived(value !== null && typeof value === 'object');
	const summary = $derived(
		isObject
			? Array.isArray(value)
				? `[${(value as unknown[]).length} items]`
				: `{${Object.keys(value as Record<string, unknown>).length} keys}`
			: String(value),
	);
</script>

{#if isObject}
	<details open={depth < 1} class="text-xs">
		<summary class="text-muted-foreground cursor-pointer font-mono">
			{name ? `${name}: ` : ''}
			{summary}
		</summary>
		<pre
			class="bg-muted/30 mt-1 max-h-64 overflow-auto rounded p-2 font-mono text-[11px] leading-snug">{JSON.stringify(
				value,
				null,
				2,
			)}</pre>
	</details>
{:else}
	<div class="font-mono text-xs">
		{#if name}<span class="text-muted-foreground">{name}: </span>{/if}
		{String(value)}
	</div>
{/if}
