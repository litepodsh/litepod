<script lang="ts">
	import PencilIcon from '@lucide/svelte/icons/pencil';
	import Trash2Icon from '@lucide/svelte/icons/trash-2';
	import BoxesIcon from '@lucide/svelte/icons/boxes';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Card from '#lib/components/ui/card/index.js';
	import ServerMark from '#lib/components/servers/server-mark.svelte';
	import type { ServerGroup } from '#lib/clients/server-group-client.js';
	import { t } from '#lib/i18n/index.svelte.js';

	let {
		group,
		canManage = false,
		onEdit,
		onDelete,
	}: {
		group: ServerGroup;
		canManage?: boolean;
		onEdit: (group: ServerGroup) => void;
		onDelete: (group: ServerGroup) => void;
	} = $props();
</script>

<Card.Root>
	<Card.Content class="flex flex-col gap-4 sm:flex-row sm:items-start sm:justify-between">
		<div class="min-w-0 space-y-3">
			<div class="flex flex-wrap items-center gap-2">
				<BoxesIcon class="size-4 shrink-0 text-muted-foreground" />
				<h2 class="truncate text-sm font-medium">{group.name}</h2>
				<Badge variant="secondary">
					{t('organizations.server_groups.member_count', { count: group.member_count })}
				</Badge>
			</div>
			{#if group.description}
				<p class="text-sm text-muted-foreground">{group.description}</p>
			{/if}
			{#if group.member_servers.length > 0}
				<div class="flex flex-wrap items-center gap-2">
					{#each group.member_servers as server (server.id)}
						<ServerMark {server} class="rounded-full border bg-muted/40 px-2 py-0.5" />
					{/each}
				</div>
			{/if}
		</div>
		{#if canManage}
			<div class="flex shrink-0 items-center gap-2">
				<Button variant="outline" size="sm" onclick={() => onEdit(group)}>
					<PencilIcon class="size-3.5" />
					{t('organizations.common.edit')}
				</Button>
				<Button
					variant="outline"
					size="sm"
					class="text-destructive hover:text-destructive"
					onclick={() => onDelete(group)}>
					<Trash2Icon class="size-3.5" />
					{t('organizations.server_groups.remove')}
				</Button>
			</div>
		{/if}
	</Card.Content>
</Card.Root>
