<script lang="ts">
	import type { Locale } from '#lib/i18n/catalogs.js';

	let { locale, european = false }: { locale: Locale; european?: boolean } = $props();
</script>

<svg viewBox="0 0 24 16" class="size-4 shrink-0 rounded-sm ring-1 ring-black/10" aria-hidden="true">
	{#if locale === 'es' && european}
		<path fill="#aa151b" d="M0 0h24v16H0z" />
		<path fill="#f1bf00" d="M0 4h24v8H0z" />
		<path fill="#aa151b" d="M5 6h2v4H5z" />
		<path fill="#f1bf00" d="M5.5 5h1v6h-1z" />
	{:else if locale === 'en' && european}
		<path fill="#012169" d="M0 0h24v16H0z" />
		<path d="m0 0 24 16M24 0 0 16" stroke="#fff" stroke-width="3" />
		<path d="m0 0 24 16M24 0 0 16" stroke="#c8102e" stroke-width="1.6" />
		<path fill="#fff" d="M9 0h6v5h9v6h-9v5H9v-5H0V5h9z" />
		<path fill="#c8102e" d="M10 0h4v6h10v4H14v6h-4v-6H0V6h10z" />
	{:else if locale === 'es'}
		<path fill="#006847" d="M0 0h8v16H0z" />
		<path fill="#fff" d="M8 0h8v16H8z" />
		<path fill="#ce1126" d="M16 0h8v16h-8z" />
		<circle cx="12" cy="8" r="1.25" fill="#8c6239" />
	{:else}
		<path fill="#b22234" d="M0 0h24v16H0z" />
		<path d="M0 2h24M0 4h24M0 6h24M0 8h24M0 10h24M0 12h24M0 14h24" stroke="#fff" stroke-width="1" />
		<path fill="#3c3b6e" d="M0 0h10v8H0z" />
		<path
			fill="#fff"
			d="M2 2h1v1H2zm3 0h1v1H5zm3 0h1v1H8zM3.5 4h1v1h-1zm3 0h1v1h-1zM2 6h1v1H2zm3 0h1v1H5zm3 0h1v1H8z" />
	{/if}
</svg>
