<script lang="ts">
	import { onMount } from 'svelte';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import AgentIcon from '#lib/components/icons/agent-icon.svelte';
	import {
		listUserMcpConnections,
		revokeUserMcpConnection,
		type McpConnection,
	} from '#lib/clients/admin-users-client.js';
	import { UseAsyncResource } from '#lib/hooks/use-async-resource.svelte.js';
	import { relativeTime } from '#lib/utils/relative-time.js';
	import { toast, toastError } from '#lib/utils/toast.js';
	import { i18n, t } from '#lib/i18n/index.svelte.js';

	let { userId }: { userId: string } = $props();

	const connections = new UseAsyncResource(
		() => listUserMcpConnections(userId),
		t('operations.users.mcp_load_failed'),
	);
	let revoking = $state<string | null>(null);

	onMount(() => connections.load());

	const utc = (time: string) => Date.parse(`${time}Z`);

	// A grant left to lapse is never marked closed, so its expiry counts too.
	function isOpen(connection: McpConnection) {
		return connection.closed_at === null && utc(connection.expires_at) > Date.now();
	}

	async function revoke(connection: McpConnection) {
		if (!confirm(t('operations.users.mcp_revoke_confirm', { client: connection.client_name }))) return;
		revoking = connection.id;
		try {
			await revokeUserMcpConnection(userId, connection.id);
			toast.success(t('operations.users.mcp_revoked'));
			await connections.load();
		} catch (error) {
			toastError(error, t('operations.users.mcp_revoke_failed'));
		} finally {
			revoking = null;
		}
	}
</script>

{#if connections.loading && !connections.data}
	<Skeleton class="h-20 w-full" />
{:else if !connections.data?.length}
	<p class="text-muted-foreground text-sm">{t('operations.users.mcp_empty')}</p>
{:else}
	<div class="overflow-x-auto rounded-md border">
		<table class="w-full text-left text-sm">
			<thead class="border-b bg-muted/40 text-muted-foreground">
				<tr>
					<th class="p-3 font-medium">{t('operations.users.mcp_agent')}</th>
					<th class="p-3 font-medium">{t('operations.users.organization')}</th>
					<th class="p-3 font-medium">{t('operations.users.status')}</th>
					<th class="p-3 font-medium">{t('operations.users.mcp_last_used')}</th>
					<th class="p-3 font-medium">{t('operations.users.created')}</th>
					<th class="p-3 font-medium">{t('operations.users.mcp_until')}</th>
					<th class="p-3"></th>
				</tr>
			</thead>
			<tbody>
				{#each connections.data as connection (connection.id)}
					{@const open = isOpen(connection)}
					<tr class="border-b last:border-0">
						<td class="p-3 font-medium">
							<span class="inline-flex items-center gap-2">
								<AgentIcon agent={connection.agent} class="size-4" />
								{connection.client_name}
							</span>
						</td>
						<td class="text-muted-foreground p-3">{connection.organization_name}</td>
						<td class="p-3">
							<Badge variant={open ? 'secondary' : 'outline'}>
								{open ? t('account.sessions.mcp_open') : t('account.sessions.closed')}
							</Badge>
						</td>
						<td class="text-muted-foreground p-3 whitespace-nowrap">
							{#if connection.last_used_at}
								<time
									datetime={`${connection.last_used_at}Z`}
									title={new Date(utc(connection.last_used_at)).toLocaleString()}>
									{relativeTime(utc(connection.last_used_at), Date.now(), i18n.locale)}
								</time>
							{:else}
								—
							{/if}
						</td>
						<td class="text-muted-foreground p-3 whitespace-nowrap">
							{new Date(utc(connection.created_at)).toLocaleString()}
						</td>
						<td class="text-muted-foreground p-3 whitespace-nowrap">
							{new Date(utc(connection.closed_at ?? connection.expires_at)).toLocaleString()}
						</td>
						<td class="p-3 text-right">
							{#if open}
								<Button
									size="sm"
									variant="destructive"
									disabled={revoking === connection.id}
									onclick={() => revoke(connection)}>
									{t('operations.users.mcp_revoke')}
								</Button>
							{/if}
						</td>
					</tr>
				{/each}
			</tbody>
		</table>
	</div>
{/if}
