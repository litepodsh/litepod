<script lang="ts">
	import type { SVGAttributes } from 'svelte/elements';

	let { class: className = '', ...restProps }: SVGAttributes<SVGSVGElement> = $props();
</script>

<svg {...restProps} class={className} viewBox="0 0 9 8" role="img" aria-label="ClickHouse"
	><path d="M0,7 h1 v1 h-1 z" fill="red" /><path d="M0,0 h1 v7 h-1 z" fill="#fc0" /><path
		d="M2,0 h1 v8 h-1 z"
		fill="#fc0" /><path d="M4,0 h1 v8 h-1 z" fill="#fc0" /><path d="M6,0 h1 v8 h-1 z" fill="#fc0" /><path
		d="M8,3.25 h1 v1.5 h-1 z"
		fill="#fc0" /></svg
>