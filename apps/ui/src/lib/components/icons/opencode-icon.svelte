<script lang="ts">
	let { class: className = 'size-3.5' }: { class?: string } = $props();
</script>

<!-- The brand ships two files that differ only in their two fixed greys: in
     both, the inner block recedes toward the page ground while the frame holds
     the foreground. `currentColor` plus an opacity says that once and works in
     either theme, so there is no light/dark pair to keep in sync here.
     The source file's mask and clipPath were both the full viewBox — no-ops
     that would have collided on `id` the moment two icons rendered together. -->
<svg class={className} viewBox="0 0 240 300" fill="currentColor" aria-hidden="true">
	<path d="M180 240H60V120H180V240Z" opacity="0.32" />
	<path d="M180 60H60V240H180V60ZM240 300H0V0H240V300Z" />
</svg>
