<script lang="ts">
	import { onMount } from 'svelte';
	import Fuse from 'fuse.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import * as Empty from '#lib/components/ui/empty/index.js';
	import { listAdminUsers, type UserWithOrgs } from '#lib/clients/admin-users-client.js';
	import { formatAuditAction } from '#lib/clients/audit-log-client.js';
	import UserDisableDialog from '#lib/components/admin/user-disable-dialog.svelte';
	import { toast } from '#lib/utils/toast.js';
	import { relativeTime } from '#lib/utils/relative-time.js';
	import SearchIcon from '@lucide/svelte/icons/search';
	import UsersIcon from '@lucide/svelte/icons/users';
	import { i18n, t } from '#lib/i18n/index.svelte.js';

	let users = $state<UserWithOrgs[]>([]);
	let loading = $state(true);
	let query = $state('');
	let toggling = $state<UserWithOrgs | null>(null);

	function message(error: unknown) {
		return error instanceof Error ? error.message : t('operations.users.request_failed');
	}

	async function refresh(silent = false) {
		if (!silent) loading = true;
		try {
			// Online first, then newest sign-ups, whatever order the API answers in.
			users = (await listAdminUsers()).sort(
				(a, b) =>
					Number(b.online) - Number(a.online) ||
					Date.parse(b.created_at) - Date.parse(a.created_at),
			);
		} catch (error) {
			if (!silent) toast.error(message(error));
		} finally {
			loading = false;
		}
	}

	onMount(() => {
		void refresh();
		const poll = setInterval(() => {
			if (!document.hidden) void refresh(true);
		}, 30_000);
		return () => clearInterval(poll);
	});

	const onlineCount = $derived(users.filter((user) => user.online).length);

	// Running work and server events aren't audit log rows, so that log's labels don't cover them.
	const ACTIVITY_LABELS: Record<string, () => string> = {
		'deployment.building': () => t('operations.users.activity_building'),
		'deployment.deploying': () => t('operations.users.activity_deploying'),
		'scheduled_task.running': () => t('operations.users.activity_task_running'),
		'system.update_applied': () => t('operations.users.activity_instance_update'),
		'server.setup_started': () => t('operations.users.activity_server_setup'),
		'server.repair_started': () => t('operations.users.activity_server_repair'),
		'server.restarted': () => t('operations.users.activity_server_restart'),
		'server.podman_restarted': () => t('operations.users.activity_podman_restart'),
	};

	const RUNNING_ACTIONS = new Set([
		'deployment.building',
		'deployment.deploying',
		'scheduled_task.running',
	]);
	const hasRunningWork = $derived(
		users.some((user) => user.recent_action && RUNNING_ACTIONS.has(user.recent_action)),
	);

	function activityLabel(action: string) {
		return ACTIVITY_LABELS[action]?.() ?? formatAuditAction(action);
	}

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
		return Number.isNaN(d.getTime()) ? iso : d.toLocaleString();
	}

	function applyDisabled(userId: string, disabled: boolean) {
		users = users.map((user) => (user.id === userId ? { ...user, disabled } : user));
	}
</script>

<div class="flex min-w-0 flex-col gap-5 px-4 py-6 sm:px-6">
	<header class="flex flex-wrap items-end justify-between gap-3">
		<div>
			<h1 class="text-xl font-semibold tracking-tight">{t('operations.users.title')}</h1>
			<p class="text-sm text-muted-foreground">
				{t('operations.users.description')}
			</p>
			{#if !loading}
				<p class="mt-1 inline-flex items-center gap-1.5 text-sm font-medium">
					<i class="size-2 rounded-full {onlineCount > 0 ? 'bg-success' : 'bg-muted-foreground/40'}"
						aria-hidden="true"></i>
					{t('operations.users.online_count', { count: onlineCount })}
				</p>
				{#if hasRunningWork}
					<p class="text-warning mt-1 text-sm">{t('operations.users.running_warning')}</p>
				{/if}
			{/if}
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
						<th class="p-3 font-medium">{t('operations.users.activity')}</th>
						<th class="p-3 font-medium">{t('operations.users.last_login')}</th>
						<th class="p-3 font-medium">{t('operations.users.usage')}</th>
						<th class="p-3 font-medium">{t('operations.users.status')}</th>
						<th class="p-3"></th>
					</tr>
				</thead>
				<tbody>
					{#each filtered as user (user.id)}
						<tr class="border-b last:border-0">
							<td class="p-3 font-medium">
								<a href="/admin/users/{user.id}" class="hover:underline">{user.name}</a>
							</td>
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
							<td class="p-3">
								{#if user.online}
									<span class="inline-flex items-center gap-1.5 text-xs font-medium">
										<i class="bg-success size-2 rounded-full" aria-hidden="true"></i>
										{t('operations.users.online')}
									</span>
									{#if user.recent_action}
										{@const action = [activityLabel(user.recent_action), user.recent_action_target]
											.filter(Boolean)
											.join(' · ')}
										<p class="text-muted-foreground mt-0.5 max-w-56 truncate text-xs" title={action}>
											{action}
										</p>
									{/if}
								{:else}
									<span class="text-muted-foreground text-xs">—</span>
								{/if}
							</td>
							<td class="text-muted-foreground p-3 whitespace-nowrap">
								{#if user.last_login_at}
									<time datetime={user.last_login_at} title={formatDate(user.last_login_at)}>
										{relativeTime(Date.parse(user.last_login_at), Date.now(), i18n.locale)}
									</time>
								{:else}
									—
								{/if}
							</td>
							<td class="text-muted-foreground p-3 text-xs whitespace-nowrap">
								{#if user.remote_servers === 0 && user.applications === 0}
									—
								{:else}
									<p>
										{t('operations.users.servers_usage', {
											ready: user.remote_servers_ready,
											total: user.remote_servers,
										})}
									</p>
									<p>
										{t('operations.users.apps_usage', {
											running: user.applications_running,
											total: user.applications,
										})}
									</p>
								{/if}
							</td>
							<td class="p-3">
								<Badge variant={user.disabled ? 'destructive' : 'secondary'}>
									{user.disabled
										? t('operations.users.status_disabled')
										: t('operations.users.status_active')}
								</Badge>
							</td>
							<td class="p-3 text-right">
								{#if !user.is_sysadmin}
									<Button
										size="sm"
										variant={user.disabled ? 'outline' : 'destructive'}
										onclick={() => (toggling = user)}>
										{user.disabled
											? t('organizations.members.enable_action')
											: t('organizations.members.disable_action')}
									</Button>
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

<UserDisableDialog bind:user={toggling} onChanged={applyDisabled} />
