<script lang="ts">
	import { onMount } from 'svelte';
	import Fuse from 'fuse.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import * as Empty from '#lib/components/ui/empty/index.js';
	import { listUsersWithOrgs, type UserWithOrgs } from '#lib/clients/auth-client.js';
	import { toast } from '#lib/utils/toast.js';
	import SearchIcon from '@lucide/svelte/icons/search';
	import UsersIcon from '@lucide/svelte/icons/users';
	import { t } from '#lib/i18n/index.svelte.js';

	let users = $state<UserWithOrgs[]>([]);
	let loading = $state(true);
	let query = $state('');

	function message(error: unknown) {
		return error instanceof Error ? error.message : t('operations.users.request_failed');
	}

	async function refresh() {
		loading = true;
		try {
			users = await listUsersWithOrgs();
		} catch (error) {
			toast.error(message(error));
		} finally {
			loading = false;
		}
	}

	onMount(() => void refresh());

	const fuse = $derived(
		new Fuse(users, {
			keys: [
				'email',
				'name',
				'organizations.name',
				'organizations.slug',
				'organizations.role_name',
			],
			threshold: 0.3,
			ignoreLocation: true,
		}),
	);

	const filtered = $derived(query.trim() ? fuse.search(query).map((r) => r.item) : users);

	function roleVariant(roleId: number) {
		if (roleId === 1) return 'default' as const;
		if (roleId === 2) return 'secondary' as const;
		return 'outline' as const;
	}

	function formatDate(iso: string) {
		if (!iso) return '—';
		const d = new Date(iso);
		return Number.isNaN(d.getTime()) ? iso : d.toLocaleDateString();
	}
</script>

<div class="flex min-w-0 flex-col gap-5 px-4 py-6 sm:px-6">
	<header class="flex flex-wrap items-end justify-between gap-3">
		<div>
			<h1 class="text-xl font-semibold tracking-tight">{t('operations.users.title')}</h1>
			<p class="text-sm text-muted-foreground">
				{t('operations.users.description')}
			</p>
		</div>
		<Button variant="outline" onclick={() => refresh()} disabled={loading}
			>{t('operations.users.refresh')}</Button>
	</header>

	<div class="relative max-w-md">
		<SearchIcon
			class="text-muted-foreground absolute left-3 top-1/2 size-4 -translate-y-1/2"
			aria-hidden="true" />
		<Input
			bind:value={query}
			type="search"
			placeholder={t('operations.users.search')}
			class="pl-9" />
	</div>

	{#if loading}
		<div class="space-y-2">
			<Skeleton class="h-10 w-full" />
			<Skeleton class="h-10 w-full" />
			<Skeleton class="h-10 w-full" />
			<Skeleton class="h-10 w-full" />
		</div>
	{:else if users.length === 0}
		<Empty.Root>
			<Empty.Header>
				<Empty.Media variant="icon">
					<UsersIcon />
				</Empty.Media>
				<Empty.Title>{t('operations.users.empty')}</Empty.Title>
				<Empty.Description>{t('operations.users.empty_description')}</Empty.Description>
			</Empty.Header>
		</Empty.Root>
	{:else if filtered.length === 0}
		<Empty.Root>
			<Empty.Header>
				<Empty.Title>{t('operations.users.no_matches')}</Empty.Title>
				<Empty.Description
					>{t('operations.users.no_matches_description', { query })}</Empty.Description>
			</Empty.Header>
		</Empty.Root>
	{:else}
		<div class="overflow-x-auto rounded-md border">
			<table class="w-full text-left text-sm">
				<thead class="border-b bg-muted/40 text-muted-foreground">
					<tr>
						<th class="p-3 font-medium">{t('operations.users.user')}</th>
						<th class="p-3 font-medium">{t('operations.users.email')}</th>
						<th class="p-3 font-medium">{t('operations.users.joined')}</th>
						<th class="p-3 font-medium">{t('operations.users.organizations')}</th>
					</tr>
				</thead>
				<tbody>
					{#each filtered as user (user.id)}
						<tr class="border-b last:border-0">
							<td class="p-3 font-medium">{user.name}</td>
							<td class="text-muted-foreground p-3">{user.email}</td>
							<td class="text-muted-foreground p-3">{formatDate(user.created_at)}</td>
							<td class="p-3">
								{#if user.organizations.length === 0}
									<span class="text-muted-foreground text-xs">—</span>
								{:else}
									<div class="flex flex-wrap gap-1.5">
										{#each user.organizations as org (org.id)}
											<Badge variant={roleVariant(org.role_id)}>
												{org.name}
												<span class="opacity-70">· {org.role_name}</span>
											</Badge>
										{/each}
									</div>
								{/if}
							</td>
						</tr>
					{/each}
				</tbody>
			</table>
		</div>
		<p class="text-muted-foreground text-xs">
			{t('operations.users.count', { filtered: filtered.length, total: users.length })}
		</p>
	{/if}
</div>
