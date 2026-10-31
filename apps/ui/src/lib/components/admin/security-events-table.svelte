<script lang="ts">
	import { onMount } from 'svelte';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import { Switch } from '#lib/components/ui/switch/index.js';
	import * as Empty from '#lib/components/ui/empty/index.js';
	import {
		listSecurityEvents,
		type SecurityEvent,
	} from '#lib/clients/admin-security-client.js';
	import { toast } from '#lib/utils/toast.js';
	import ShieldCheckIcon from '@lucide/svelte/icons/shield-check';
	import { t } from '#lib/i18n/index.svelte.js';

	const PAGE = 100;

	let {
		userId,
		flagged = $bindable(false),
	}: {
		/** Only this user's events; also hides the user column. */
		userId?: string;
		flagged?: boolean;
	} = $props();

	let events = $state<SecurityEvent[]>([]);
	let loading = $state(true);
	let loadingMore = $state(false);
	let hasMore = $state(false);
	let ip = $state('');

	async function fetchPage(before?: string) {
		const page = await listSecurityEvents({
			user_id: userId,
			ip: ip.trim(),
			flagged,
			before,
			limit: PAGE,
		});
		hasMore = page.length === PAGE;
		return page;
	}

	async function refresh() {
		loading = true;
		try {
			events = await fetchPage();
		} catch (error) {
			toast.error(error instanceof Error ? error.message : t('operations.security.request_failed'));
		} finally {
			loading = false;
		}
	}

	async function more() {
		loadingMore = true;
		try {
			events = [...events, ...(await fetchPage(events.at(-1)?.id))];
		} catch (error) {
			toast.error(error instanceof Error ? error.message : t('operations.security.request_failed'));
		} finally {
			loadingMore = false;
		}
	}

	onMount(() => void refresh());

	function filterByIp(value: string) {
		ip = value;
		void refresh();
	}

	function formatDate(value: string) {
		// SQLite `datetime('now')` is UTC without a zone marker.
		const d = new Date(`${value.replace(' ', 'T')}Z`);
		return Number.isNaN(d.getTime()) ? value : d.toLocaleString();
	}

	function place(event: SecurityEvent) {
		return [event.city, event.country].filter(Boolean).join(', ');
	}

	/** Failed logins carry the attempted email in `metadata`, not a user id. */
	function subject(event: SecurityEvent) {
		if (event.user_email) return event.user_name || event.user_email;
		return event.action === 'auth.login_failed' ? event.metadata : null;
	}
</script>

<div class="space-y-3">
	<form
		class="flex flex-wrap items-center gap-3"
		onsubmit={(e) => {
			e.preventDefault();
			void refresh();
		}}>
		<Input
			bind:value={ip}
			type="search"
			placeholder={t('operations.security.filter_ip')}
			class="max-w-56" />
		<label class="text-muted-foreground flex items-center gap-2 text-sm">
			<Switch bind:checked={() => flagged, (value) => ((flagged = value), void refresh())} />
			{t('operations.security.only_flagged')}
		</label>
		<Button type="submit" variant="outline" size="sm" disabled={loading}
			>{t('operations.users.refresh')}</Button>
	</form>

	{#if loading}
		<div class="space-y-2">
			<Skeleton class="h-10 w-full" />
			<Skeleton class="h-10 w-full" />
			<Skeleton class="h-10 w-full" />
		</div>
	{:else if events.length === 0}
		<Empty.Root>
			<Empty.Header>
				<Empty.Media variant="icon">
					<ShieldCheckIcon />
				</Empty.Media>
				<Empty.Title>{t('operations.security.empty')}</Empty.Title>
				<Empty.Description>{t('operations.security.empty_description')}</Empty.Description>
			</Empty.Header>
		</Empty.Root>
	{:else}
		<div class="overflow-x-auto rounded-md border">
			<table class="w-full text-left text-sm">
				<thead class="border-b bg-muted/40 text-muted-foreground">
					<tr>
						<th class="p-3 font-medium">{t('operations.security.when')}</th>
						<th class="p-3 font-medium">{t('operations.security.action')}</th>
						{#if !userId}
							<th class="p-3 font-medium">{t('operations.users.user')}</th>
						{/if}
						<th class="p-3 font-medium">{t('operations.security.ip')}</th>
						<th class="p-3 font-medium">{t('operations.security.location')}</th>
						<th class="p-3 font-medium">{t('operations.security.device')}</th>
					</tr>
				</thead>
				<tbody>
					{#each events as event (event.id)}
						<tr class="border-b last:border-0 {event.flag ? 'bg-destructive/5' : ''}">
							<td class="text-muted-foreground p-3 whitespace-nowrap">
								{formatDate(event.created_at)}
							</td>
							<td class="p-3">
								<div class="flex flex-wrap items-center gap-1.5">
									<code class="text-xs">{event.action}</code>
									{#if event.flag}
										<Badge variant="destructive">{t(`operations.security.flag_${event.flag}`)}</Badge>
									{/if}
								</div>
								{#if event.actor_user_id && event.actor_user_id !== event.user_id}
									<p class="text-muted-foreground mt-0.5 text-xs">
										{t('operations.security.by', {
											actor: event.actor_token_name ?? event.actor_email ?? event.actor_user_id,
										})}
									</p>
								{/if}
							</td>
							{#if !userId}
								<td class="p-3">
									{#if event.user_id}
										<a href="/admin/users/{event.user_id}" class="font-medium hover:underline"
											>{subject(event) ?? event.user_id}</a>
									{:else}
										<span class="text-muted-foreground">{subject(event) ?? '—'}</span>
									{/if}
								</td>
							{/if}
							<td class="p-3 font-mono text-xs">
								{#if event.source_ip}
									<button
										type="button"
										class="hover:underline"
										title={t('operations.security.filter_this_ip')}
										onclick={() => filterByIp(event.source_ip ?? '')}>{event.source_ip}</button>
								{:else}
									—
								{/if}
							</td>
							<td class="text-muted-foreground p-3" title={event.asn ?? undefined}>
								{place(event) || '—'}
							</td>
							<td class="text-muted-foreground max-w-64 truncate p-3 text-xs" title={event.user_agent ?? undefined}>
								{event.user_agent ?? '—'}
							</td>
						</tr>
					{/each}
				</tbody>
			</table>
		</div>
		{#if hasMore}
			<Button variant="outline" size="sm" onclick={() => more()} disabled={loadingMore}
				>{t('operations.security.load_more')}</Button>
		{/if}
	{/if}
</div>
