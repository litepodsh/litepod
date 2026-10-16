<script lang="ts">
	import BoxesIcon from '@lucide/svelte/icons/boxes';
	import type { ServerGroupSummaryDto } from '#lib/clients/project-client.js';

	let { group, class: className = '' }: { group: ServerGroupSummaryDto; class?: string } = $props();
</script>

<span
	class="flex min-w-0 items-center gap-1.5 text-xs text-muted-foreground {className}"
	title={group.name}>
	<BoxesIcon class="size-3.5 shrink-0" />
	<span class="truncate">{group.name}</span>
</span>
