<script lang="ts">
	import type { Snippet } from 'svelte';
	import { mergeProps } from 'bits-ui';
	import * as Collapsible from '#lib/components/ui/collapsible/index.js';
	import * as Tooltip from '#lib/components/ui/tooltip/index.js';

	let { label, class: className = '', children }: { label: string; class?: string; children: Snippet } =
		$props();
</script>

<!-- The chevron that opens a tree branch, with a tooltip naming what it does. -->
<Tooltip.Root>
	<Tooltip.Trigger>
		{#snippet child({ props: tooltipProps })}
			<Collapsible.Trigger>
				{#snippet child({ props: triggerProps })}
					<button {...mergeProps(tooltipProps, triggerProps)} aria-label={label} class={className}>
						{@render children()}
					</button>
				{/snippet}
			</Collapsible.Trigger>
		{/snippet}
	</Tooltip.Trigger>
	<Tooltip.Content side="right">{label}</Tooltip.Content>
</Tooltip.Root>
