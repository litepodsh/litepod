<script lang="ts">
	let { class: className = 'size-3.5' }: { class?: string } = $props();

	// The circle-of-friends marks are one shape reused at three rotations, so the id
	// must be unique per instance for two icons on the same page not to resolve to
	// each other's definition.
	const uid = $props.id();
	const markId = `ubuntu-mark-${uid}`;
</script>

<svg class={className} viewBox="0 0 100 100" aria-hidden="true">
	<circle cx="50" cy="50" r="45" fill="#f47421" />
	<circle cx="50" cy="50" r="21.8" fill="none" stroke="#fff" stroke-width="8.6" />
	<g id={markId}>
		<circle cx="19.4" cy="50" r="8.4" fill="#f47421" />
		<path stroke="#f47421" stroke-width="3.2" d="M67 50h10" />
		<circle cx="19.4" cy="50" r="6" fill="#fff" />
	</g>
	<use href={`#${markId}`} transform="rotate(120 50 50)" />
	<use href={`#${markId}`} transform="rotate(240 50 50)" />
</svg>
