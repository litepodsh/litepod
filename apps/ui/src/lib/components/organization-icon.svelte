<script lang="ts">
	import { resolveOrganizationIcon, isOrganizationIconUrl } from '#lib/utils/organization-icons.js';
	import { contrastColor } from '#lib/utils/color.js';

	let {
		icon,
		bg = '',
		class: className = 'size-4',
	}: { icon: string; bg?: string; class?: string } = $props();

	const isUrl = $derived(isOrganizationIconUrl(icon || ''));
	const Comp = $derived(resolveOrganizationIcon(icon || ''));
	const fg = $derived(bg ? contrastColor(bg) : '');
</script>

{#if bg}
	<div
		class="flex size-full items-center justify-center"
		style="background-color: {bg}; color: {fg}">
		{#if isUrl}
			<img src={icon} alt="" loading="lazy" class="{className} object-contain" />
		{:else}
			<Comp class={className} />
		{/if}
	</div>
{:else if isUrl}
	<img src={icon} alt="" class="{className} object-contain" />
{:else}
	<Comp class={className} />
{/if}
