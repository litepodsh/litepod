<script lang="ts">
	import { page } from '$app/state';
	import { onMount } from 'svelte';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import UserDisableDialog from '#lib/components/admin/user-disable-dialog.svelte';
	import { listUsersWithOrgs, type UserWithOrgs } from '#lib/clients/auth-client.js';
	import { toast } from '#lib/utils/toast.js';
	import ArrowLeftIcon from '@lucide/svelte/icons/arrow-left';
	import { t } from '#lib/i18n/index.svelte.js';

	let user = $state<UserWithOrgs | null>(null);
	let loading = $state(true);
	let toggling = $state<UserWithOrgs | null>(null);

	onMount(async () => {
		try {
			const users = await listUsersWithOrgs();
			user = users.find((candidate) => candidate.id === page.params.user_id) ?? null;
		} catch (error) {
			toast.error(error instanceof Error ? error.message : t('operations.users.request_failed'));
		} finally {
			loading = false;
		}
	});

	function formatDate(iso: string) {
		if (!iso) return '—';
		const d = new Date(iso);
		return Number.isNaN(d.getTime()) ? iso : d.toLocaleString();
	}

	function applyDisabled(_userId: string, disabled: boolean) {
		if (user) user = { ...user, disabled };
	}
</script>

<div class="flex min-w-0 flex-col gap-5 px-4 py-6 sm:px-6">
	<a
		href="/admin/users"
		class="text-muted-foreground hover:text-foreground inline-flex w-fit items-center gap-1.5 text-sm">
		<ArrowLeftIcon class="size-4" />
		{t('operations.users.title')}
	</a>

	{#if loading}
		<Skeleton class="h-24 w-full" />
		<Skeleton class="h-40 w-full" />
	{:else if !user}
		<p class="text-muted-foreground text-sm">{t('operations.users.not_found')}</p>
	{:else}
		<header class="flex flex-wrap items-start justify-between gap-3">
			<div class="min-w-0">
				<h1 class="truncate text-xl font-semibold tracking-tight">{user.name}</h1>
				<p class="text-muted-foreground text-sm">{user.email}</p>
			</div>
			<div class="flex items-center gap-2">
				<Badge variant={user.disabled ? 'destructive' : 'secondary'}>
					{user.disabled
						? t('operations.users.status_disabled')
						: t('operations.users.status_active')}
				</Badge>
				{#if user.is_sysadmin}
					<Badge variant="default">{t('operations.users.sysadmin')}</Badge>
				{:else}
					<Button
						size="sm"
						variant={user.disabled ? 'outline' : 'destructive'}
						onclick={() => (toggling = user)}>
						{user.disabled
							? t('organizations.members.enable_action')
							: t('organizations.members.disable_action')}
					</Button>
				{/if}
			</div>
		</header>

		<dl class="grid gap-3 rounded-md border p-4 text-sm sm:grid-cols-2">
			<div>
				<dt class="text-muted-foreground text-xs">{t('operations.users.created')}</dt>
				<dd class="font-medium">{formatDate(user.created_at)}</dd>
			</div>
			<div>
				<dt class="text-muted-foreground text-xs">{t('operations.users.organizations')}</dt>
				<dd class="font-medium">{user.organizations.length}</dd>
			</div>
			<div class="sm:col-span-2">
				<dt class="text-muted-foreground text-xs">ID</dt>
				<dd class="font-mono text-xs">{user.id}</dd>
			</div>
		</dl>

		<section class="space-y-2">
			<h2 class="text-sm font-medium">{t('operations.users.organizations')}</h2>
			{#if user.organizations.length === 0}
				<p class="text-muted-foreground text-sm">{t('operations.users.no_organizations')}</p>
			{:else}
				<div class="overflow-x-auto rounded-md border">
					<table class="w-full text-left text-sm">
						<thead class="border-b bg-muted/40 text-muted-foreground">
							<tr>
								<th class="p-3 font-medium">{t('operations.users.organization')}</th>
								<th class="p-3 font-medium">{t('operations.users.role')}</th>
								<th class="p-3 font-medium">{t('operations.users.joined')}</th>
							</tr>
						</thead>
						<tbody>
							{#each [...user.organizations].sort((a, b) => Date.parse(b.joined_at) - Date.parse(a.joined_at)) as org (org.id)}
								<tr class="border-b last:border-0">
									<td class="p-3">
										<span class="font-medium">{org.name}</span>
										<span class="text-muted-foreground ml-1 text-xs">{org.slug}</span>
									</td>
									<td class="p-3"><Badge variant="outline">{org.role_name}</Badge></td>
									<td class="text-muted-foreground p-3">{formatDate(org.joined_at)}</td>
								</tr>
							{/each}
						</tbody>
					</table>
				</div>
			{/if}
		</section>
	{/if}
</div>

<UserDisableDialog bind:user={toggling} onChanged={applyDisabled} />
