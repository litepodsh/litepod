<script lang="ts">
	/** A settings block: title and description on the left, the form on the right.
	 *  Stacks below `md`. */
	import type { Snippet } from 'svelte';

	let {
		title,
		description,
		aside,
		children,
	}: {
		title: string;
		description?: string;
		/** Extra content under the description, e.g. a status badge. */
		aside?: Snippet;
		children: Snippet;
	} = $props();
</script>

<section
	class="lp-panel grid gap-3.5 rounded-[15px] border border-foreground/9 bg-card p-4 text-sm text-card-foreground sm:p-5 md:grid-cols-[220px_minmax(0,1fr)] md:gap-6">
	<div class="min-w-0">
		<h2 class="text-[15px] font-semibold">{title}</h2>
		{#if description}
			<p class="mt-1 text-[12.5px] text-muted-foreground">{description}</p>
		{/if}
		{@render aside?.()}
	</div>
	<div class="flex min-w-0 flex-col gap-4">{@render children()}</div>
</section>
