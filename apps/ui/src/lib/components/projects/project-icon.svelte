<script lang="ts">
	import FolderKanbanIcon from '@lucide/svelte/icons/folder-kanban';
	import { isDisplayableImageUrl } from '#lib/utils/application-card-icon.js';
	import { isNamedIcon } from '#lib/utils/organization-icons.js';
	import OrganizationIcon from '#lib/components/organization-icon.svelte';

	let { iconUrl }: { iconUrl: string | null | undefined } = $props();

	let failed = $state(false);
</script>

{#if isNamedIcon(iconUrl)}
	<OrganizationIcon icon={iconUrl} class="size-4 shrink-0" />
{:else if isDisplayableImageUrl(iconUrl) && !failed}
	<img
		src={iconUrl}
		alt=""
		class="size-4 shrink-0 rounded-[4px] object-contain"
		onerror={() => (failed = true)} />
{:else}
	<FolderKanbanIcon class="text-muted-foreground size-3.5 shrink-0" />
{/if}
