<script lang="ts">
	let { class: className = 'size-3.5' }: { class?: string } = $props();
</script>

<!-- Forgejo logo by Caesar Schinas, CC BY-SA 4.0. Styles inlined so the SVG's element selectors can't leak globally. -->
<svg class={className} viewBox="0 0 212 212" xmlns="http://www.w3.org/2000/svg" aria-hidden="true">
	<g transform="translate(6,6)" fill="none">
		<path d="M58 168 v-98 a50 50 0 0 1 50-50 h20" stroke="#ff6600" stroke-width="25" />
		<path d="M58 168 v-30 a50 50 0 0 1 50-50 h20" stroke="#d40000" stroke-width="25" />
		<circle cx="142" cy="20" r="18" stroke="#ff6600" stroke-width="15" />
		<circle cx="142" cy="88" r="18" stroke="#d40000" stroke-width="15" />
		<circle cx="58" cy="180" r="18" stroke="#d40000" stroke-width="15" />
	</g>
</svg>
