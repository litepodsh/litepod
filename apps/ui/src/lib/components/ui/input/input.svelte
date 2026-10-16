<script lang="ts">
	import type { HTMLInputAttributes, HTMLInputTypeAttribute } from 'svelte/elements';
	import { cn, type WithElementRef } from '#lib/utils.js';

	type InputType = Exclude<HTMLInputTypeAttribute, 'file'>;

	type Props = WithElementRef<
		Omit<HTMLInputAttributes, 'type'> &
			({ type: 'file'; files?: FileList } | { type?: InputType; files?: undefined })
	>;

	let {
		ref = $bindable(null),
		value = $bindable(),
		type,
		files = $bindable(),
		class: className,
		'data-slot': dataSlot = 'input',
		...restProps
	}: Props = $props();
</script>

{#if type === 'file'}
	<input
		bind:this={ref}
		data-slot={dataSlot}
		class={cn(
			'lp-well in-data-[slot=input-group]:shadow-none bg-[color-mix(in_srgb,var(--lp-deep)_55%,var(--card))] focus-visible:border-ring focus-visible:ring-ring/30 aria-invalid:ring-destructive/20 dark:aria-invalid:ring-destructive/40 aria-invalid:border-destructive dark:aria-invalid:border-destructive/50 h-9 rounded-[10px] border border-foreground/14 px-3 py-1 text-base transition-[color,box-shadow,transform] duration-300 ease-[var(--lp-ease)] file:h-6 file:text-sm file:font-medium focus-visible:ring-3 aria-invalid:ring-3 md:text-sm file:text-foreground placeholder:text-muted-foreground w-full min-w-0 outline-none file:inline-flex file:border-0 file:bg-transparent disabled:pointer-events-none disabled:cursor-not-allowed disabled:opacity-50',
			className,
		)}
		type="file"
		bind:files
		bind:value
		{...restProps} />
{:else}
	<input
		bind:this={ref}
		data-slot={dataSlot}
		class={cn(
			'lp-well in-data-[slot=input-group]:shadow-none bg-[color-mix(in_srgb,var(--lp-deep)_55%,var(--card))] focus-visible:border-ring focus-visible:ring-ring/30 aria-invalid:ring-destructive/20 dark:aria-invalid:ring-destructive/40 aria-invalid:border-destructive dark:aria-invalid:border-destructive/50 h-9 rounded-[10px] border border-foreground/14 px-3 py-1 text-base transition-[color,box-shadow,transform] duration-300 ease-[var(--lp-ease)] file:h-6 file:text-sm file:font-medium focus-visible:ring-3 aria-invalid:ring-3 md:text-sm file:text-foreground placeholder:text-muted-foreground w-full min-w-0 outline-none file:inline-flex file:border-0 file:bg-transparent disabled:pointer-events-none disabled:cursor-not-allowed disabled:opacity-50',
			className,
		)}
		{type}
		bind:value
		{...restProps} />
{/if}
