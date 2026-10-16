<script lang="ts">
	import { onMount } from 'svelte';
	import CopyIcon from '@lucide/svelte/icons/copy';
	import PlusIcon from '@lucide/svelte/icons/plus';
	import RotateCcwIcon from '@lucide/svelte/icons/rotate-ccw';
	import UserPlusIcon from '@lucide/svelte/icons/user-plus';
	import XIcon from '@lucide/svelte/icons/x';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Dialog from '#lib/components/ui/dialog/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Label } from '#lib/components/ui/label/index.js';
	import * as Select from '#lib/components/ui/select/index.js';
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import {
		createInvitation,
		invitationsForOrganization,
		listInvitations,
		organizationInvitationInput,
		regenerateInvitation,
		revokeInvitation,
		toInvitationUrl,
		type Invitation,
	} from '#lib/clients/invitation-client.js';
	import { t } from '#lib/i18n/index.svelte.js';
	import { toast } from '#lib/utils/toast.js';

	let {
		organizationId,
		organizationName,
	}: {
		organizationId: string;
		organizationName: string;
	} = $props();

	let invitations = $state<Invitation[]>([]);
	let username = $state('');
	let email = $state('');
	let roleId = $state(3);
	let loading = $state(true);
	let saving = $state(false);
	let actionId = $state<string | null>(null);
	let latestUrl = $state('');
	let formError = $state('');
	let dialogOpen = $state(false);
	let wasOpen = $state(false);

	const roleHelp = $derived(
		roleId === 1
			? t('account.invitations.role_help_admin')
			: roleId === 2
				? t('account.invitations.role_help_editor')
				: t('account.invitations.role_help_viewer'),
	);
	const roleLabel = $derived(
		roleId === 1
			? t('account.invitations.role_admin')
			: roleId === 2
				? t('account.invitations.role_editor')
				: t('account.invitations.role_viewer'),
	);

	$effect(() => {
		if (wasOpen && !dialogOpen) resetForm();
		wasOpen = dialogOpen;
	});

	onMount(async () => {
		await refresh();
	});

	function resetForm() {
		username = '';
		email = '';
		roleId = 3;
		latestUrl = '';
		formError = '';
	}

	async function refresh() {
		try {
			invitations = invitationsForOrganization(await listInvitations(), organizationId);
		} catch (err) {
			toast.error(err instanceof Error ? err.message : t('account.invitations.load_failed'));
		} finally {
			loading = false;
		}
	}

	function valid() {
		return username.trim().length > 0 && email.includes('@') && [1, 2, 3].includes(roleId);
	}

	async function copy(url: string) {
		try {
			await navigator.clipboard.writeText(url);
			toast.success(t('account.invitations.copy_success'));
		} catch {
			toast.error(t('account.invitations.copy_failed'));
		}
	}

	async function submit(event: SubmitEvent) {
		event.preventDefault();
		formError = '';
		if (!valid()) {
			formError = t('account.invitations.error_required');
			return;
		}
		if (saving) return;
		saving = true;
		try {
			const created = await createInvitation(
				organizationInvitationInput(organizationId, username, email, roleId),
			);
			latestUrl = toInvitationUrl(created.token);
			await refresh();
			toast.success(t('account.invitations.created_toast', { organization: organizationName }));
		} catch (err) {
			formError = err instanceof Error ? err.message : t('account.invitations.create_failed');
		} finally {
			saving = false;
		}
	}

	async function revoke(invitation: Invitation) {
		actionId = invitation.id;
		try {
			await revokeInvitation(invitation.id);
			await refresh();
			toast.success(t('account.invitations.revoked'));
		} catch (err) {
			toast.error(err instanceof Error ? err.message : t('account.invitations.revoke_failed'));
		} finally {
			actionId = null;
		}
	}

	async function regenerate(invitation: Invitation) {
		actionId = invitation.id;
		try {
			const created = await regenerateInvitation(invitation.id);
			await copy(toInvitationUrl(created.token));
			await refresh();
		} catch (err) {
			toast.error(err instanceof Error ? err.message : t('account.invitations.regenerate_failed'));
		} finally {
			actionId = null;
		}
	}
</script>

<section class="space-y-3" aria-labelledby="invitations-heading">
	<div class="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
		<div>
			<h3 id="invitations-heading" class="text-[15px] font-semibold">
				{t('account.invitations.title')}
				<span class="text-muted-foreground">({invitations.length})</span>
			</h3>
			<p class="mt-1 text-sm text-muted-foreground">
				{t('account.invitations.description')}
			</p>
		</div>
		<Button class="sm:shrink-0" onclick={() => (dialogOpen = true)}>
			<PlusIcon class="size-4" />
			{t('account.invitations.invite_member')}
		</Button>
	</div>

	{#if loading}
		<div class="lp-raised overflow-hidden rounded-2xl border bg-card">
			{#each [0, 1] as row (row)}
				<div class="flex items-center gap-3 p-4 not-last:border-b not-last:border-border/60">
					<Skeleton class="size-9 rounded-lg" />
					<div class="flex-1 space-y-2">
						<Skeleton class="h-4 w-36" />
						<Skeleton class="h-3 w-52" />
					</div>
				</div>
			{/each}
		</div>
	{:else if invitations.length === 0}
		<div class="flex flex-col items-start gap-3 rounded-2xl border border-dashed border-foreground/15 bg-card p-5">
			<div
				class="flex size-9 items-center justify-center lp-raised rounded-xl border bg-muted">
				<UserPlusIcon class="size-4 text-muted-foreground" />
			</div>
			<div>
				<p class="font-medium">{t('account.invitations.empty_title')}</p>
				<p class="mt-1 text-sm text-muted-foreground">
					{t('account.invitations.empty_description', { organization: organizationName })}
				</p>
			</div>
		</div>
	{:else}
		<ul class="lp-raised overflow-hidden rounded-2xl border bg-card">
			{#each invitations as invitation (invitation.id)}
				<li
					class="flex flex-col gap-3 p-4 not-last:border-b not-last:border-border/60 sm:flex-row sm:items-center">
					<div class="min-w-0 flex-1">
						<div class="flex flex-wrap items-center gap-2">
							<p class="truncate font-medium">{invitation.username}</p>
							<Badge variant="secondary" class="capitalize">{invitation.status}</Badge>
						</div>
						<p class="mt-1 break-words text-sm text-muted-foreground">{invitation.email}</p>
						<div class="mt-2 flex flex-wrap gap-x-4 gap-y-1 text-xs text-muted-foreground">
							<span
								>{invitation.organizations[0]?.role_name ??
									t('account.invitations.unknown_role')}</span>
							<span>{t('account.invitations.expires', { date: invitation.expires_at })} UTC</span>
						</div>
					</div>
					<div class="flex flex-wrap gap-2 sm:shrink-0">
						{#if invitation.status !== 'accepted'}
							<Button
								size="sm"
								variant="outline"
								disabled={actionId === invitation.id}
								onclick={() => regenerate(invitation)}>
								<RotateCcwIcon class="size-4" />
								{t('account.invitations.regenerate')}
							</Button>
						{/if}
						{#if invitation.status === 'pending'}
							<Button
								size="sm"
								variant="ghost"
								class="text-destructive hover:text-destructive"
								disabled={actionId === invitation.id}
								onclick={() => revoke(invitation)}>
								<XIcon class="size-4" />
								{t('account.common.revoke')}
							</Button>
						{/if}
					</div>
				</li>
			{/each}
		</ul>
	{/if}
</section>

<Dialog.Root bind:open={dialogOpen}>
	<Dialog.Content class="sm:max-w-lg">
		<Dialog.Header>
			<Dialog.Title>{t('account.invitations.invite_member')}</Dialog.Title>
			<Dialog.Description>
				{t('account.invitations.dialog_description', { organization: organizationName })}
			</Dialog.Description>
		</Dialog.Header>

		<form onsubmit={submit} class="space-y-5">
			<div class="grid gap-4 sm:grid-cols-2">
				<div class="grid gap-2">
					<Label for="invitation-username">{t('account.invitations.username')}</Label>
					<Input id="invitation-username" bind:value={username} maxlength={100} autofocus />
				</div>
				<div class="grid gap-2">
					<Label for="invitation-email">{t('account.profile.email')}</Label>
					<Input id="invitation-email" type="email" bind:value={email} autocomplete="email" />
				</div>
			</div>

			<div class="grid gap-2">
				<Label for="invitation-role">{t('account.invitations.role')}</Label>
				<Select.Root
					type="single"
					value={String(roleId)}
					onValueChange={(value) => {
						const nextRole = Number(value);
						if ([1, 2, 3].includes(nextRole)) roleId = nextRole;
					}}>
					<Select.Trigger
						id="invitation-role"
						class="w-full"
						aria-describedby="invitation-role-help">
						{roleLabel}
					</Select.Trigger>
					<Select.Content>
						<Select.Item value="1" label={t('account.invitations.role_admin')}
							>{t('account.invitations.role_admin')}</Select.Item>
						<Select.Item value="2" label={t('account.invitations.role_editor')}
							>{t('account.invitations.role_editor')}</Select.Item>
						<Select.Item value="3" label={t('account.invitations.role_viewer')}
							>{t('account.invitations.role_viewer')}</Select.Item>
					</Select.Content>
				</Select.Root>
				<p id="invitation-role-help" class="text-xs text-muted-foreground">{roleHelp}</p>
			</div>

			{#if formError}
				<p role="alert" class="rounded-lg bg-destructive/10 p-3 text-sm text-destructive">
					{formError}
				</p>
			{/if}

			{#if latestUrl}
				<div class="grid gap-2">
					<Label for="invitation-link">{t('account.invitations.invitation_link')}</Label>
					<div class="flex flex-col gap-2 sm:flex-row">
						<Input id="invitation-link" readonly value={latestUrl} class="min-w-0 flex-1" />
						<Button type="button" variant="outline" onclick={() => copy(latestUrl)}>
							<CopyIcon class="size-4" />
							{t('account.invitations.copy_link')}
						</Button>
					</div>
				</div>
			{/if}

			<Dialog.Footer>
				<Button type="button" variant="outline" onclick={() => (dialogOpen = false)}
					>{t('account.common.cancel')}</Button>
				<Button type="submit" disabled={!valid() || saving || Boolean(latestUrl)}>
					{saving
						? t('account.invitations.creating')
						: latestUrl
							? t('account.invitations.created')
							: t('account.invitations.create')}
				</Button>
			</Dialog.Footer>
		</form>
	</Dialog.Content>
</Dialog.Root>
