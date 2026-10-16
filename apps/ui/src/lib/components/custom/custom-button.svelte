<script lang="ts">
	import type { Snippet } from 'svelte';
	import { Button, type ButtonProps } from '#lib/components/ui/button/index.js';
	import { cn } from '#lib/utils.js';

	let {
		class: className,
		icon,
		children,
		...restProps
	}: ButtonProps & { icon?: Snippet } = $props();
</script>

<Button {...restProps} class={cn('group/custom-button', className)}>
	{@render children?.()}
	{#if icon}
		<span
			data-slot="custom-button-icon"
			class="bg-foreground/10 grid size-7 place-items-center rounded-full transition-transform duration-300 ease-[var(--lp-ease)] group-hover/custom-button:translate-x-0.5 group-hover/custom-button:-translate-y-px">
			{@render icon()}
		</span>
	{/if}
</Button>
