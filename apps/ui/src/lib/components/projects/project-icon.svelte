<script lang="ts">
	import FolderKanbanIcon from '@lucide/svelte/icons/folder-kanban';
	import { isDisplayableImageUrl } from '#lib/utils/application-card-icon.js';

	let { iconUrl }: { iconUrl: string | null | undefined } = $props();

	let failed = $state(false);
</script>

{#if isDisplayableImageUrl(iconUrl) && !failed}
	<img
		src={iconUrl}
		alt=""
		class="size-4 shrink-0 rounded-[4px] object-contain"
		onerror={() => (failed = true)} />
{:else}
	<FolderKanbanIcon class="text-muted-foreground size-3.5 shrink-0" />
{/if}
