<script lang="ts">
	let { value, label = 'Live data' }: { value: unknown; label?: string } = $props();
	let formatted = $derived(JSON.stringify(value, null, 2));
</script>

<section class="overflow-hidden rounded-2xl border bg-muted/20">
	<div
		class="border-b px-4 py-2 text-xs font-medium uppercase tracking-wider text-muted-foreground">
		{label}
	</div>
	<pre
		class="max-h-[32rem] overflow-auto p-4 text-xs leading-relaxed text-foreground">{formatted}</pre>
</section>
