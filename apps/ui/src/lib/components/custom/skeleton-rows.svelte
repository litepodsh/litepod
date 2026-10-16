<script lang="ts">
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import { cn } from '#lib/utils.js';

	/**
	 * Placeholder that occupies the shape of the content still loading, so the
	 * panel does not collapse to a one-line "Loading…" and then shove everything
	 * down when data lands.
	 *
	 * Pick `rows` and `height` to match what actually renders: a list of server
	 * cards is taller than a table of audit rows.
	 */
	let {
		rows = 4,
		height = 'h-14',
		label = 'Loading',
		class: className,
	}: {
		rows?: number;
		/** Tailwind height class for one row, e.g. `h-14`, `h-64`. */
		height?: string;
		/** Announced to screen readers in place of the visual placeholder. */
		label?: string;
		class?: string;
	} = $props();
</script>

<div class={cn('space-y-2', className)} role="status" aria-busy="true" aria-label={label}>
	{#each { length: rows } as _, index (index)}
		<Skeleton class={height} />
	{/each}
</div>
