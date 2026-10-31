<script lang="ts">
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Avatar from '#lib/components/ui/avatar/index.js';
	import * as AlertDialog from '#lib/components/ui/alert-dialog/index.js';
	import DeleteConfirmControl from '#lib/components/custom/delete-confirm-control.svelte';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import UsersIcon from '@lucide/svelte/icons/users';
	import Trash2Icon from '@lucide/svelte/icons/trash-2';
	import { Switch } from '#lib/components/ui/switch/index.js';
	import { NativeSelect } from '#lib/components/ui/native-select/index.js';
	import EffectivePermissionsDialog from '#lib/components/access/effective-permissions-dialog.svelte';
	import ShieldIcon from '@lucide/svelte/icons/shield';
	import {
		removeMember,
		setUserDisabled,
		updateMemberRole,
		type OrganizationMemberDto,
	} from '#lib/clients/organization-client.js';
	import { toast } from '#lib/utils/toast.js';
	import { t } from '#lib/i18n/index.svelte.js';

	let {
		organizationId,
		members = $bindable<OrganizationMemberDto[]>([]),
		canManageMembers,
		isSysadmin = false,
		myUserId = null,
	}: {
		organizationId: string;
		members?: OrganizationMemberDto[];
		canManageMembers: boolean;
		isSysadmin?: boolean;
		myUserId?: string | null;
	} = $props();

	let removingId = $state<string | null>(null);
	let confirmMember = $state<OrganizationMemberDto | null>(null);
	let inspecting = $state<OrganizationMemberDto | null>(null);
	/** Member whose enabled switch was flipped, awaiting confirmation. */
	let toggling = $state<OrganizationMemberDto | null>(null);
	let togglingBusy = $state(false);

	async function confirmToggle() {
		const member = toggling;
		if (!member) return;
		const disabled = !member.disabled;
		togglingBusy = true;
		try {
			await setUserDisabled(member.user_id, disabled);
			members = members.map((m) => (m.user_id === member.user_id ? { ...m, disabled } : m));
			const name = member.name || member.email;
			toast.success(
				t(disabled ? 'organizations.members.disabled_done' : 'organizations.members.enabled_done', {
					name,
				}),
			);
		} catch (err) {
			toast.error(err instanceof Error ? err.message : t('organizations.members.toggle_failed'));
		} finally {
			togglingBusy = false;
			toggling = null;
		}
	}

	/** Base roles; finer access is granted on the Access tab. */
	const BASE_ROLES: [number, string][] = [
		[1, 'Admin'],
		[2, 'Editor'],
		[3, 'Viewer'],
		[4, 'Member'],
	];

	async function changeRole(member: OrganizationMemberDto, select: HTMLSelectElement) {
		const roleId = Number(select.value);
		try {
			await updateMemberRole(organizationId, member.user_id, roleId);
			members = members.map((m) =>
				m.user_id === member.user_id
					? { ...m, role_id: roleId, role_name: BASE_ROLES.find(([id]) => id === roleId)![1] }
					: m,
			);
			toast.success(t('organizations.members.role_changed'));
		} catch (err) {
			// Refused (e.g. last admin): put the select back.
			select.value = String(member.role_id);
			toast.error(
				err instanceof Error ? err.message : t('organizations.members.role_change_failed'),
			);
		}
	}

	function avatarUrl(name: string) {
		const seed = name.replace(/[^a-zA-Z0-9]/g, '').slice(0, 20) || 'user';
		const params = new URLSearchParams({ theme: 'micah', seed, size: '64' });
		return `https://avatune.sebasgc.xyz/?${params}`;
	}

	function initials(name: string) {
		return name.trim().slice(0, 2).toUpperCase() || '?';
	}

	async function handleRemove(member: OrganizationMemberDto) {
		removingId = member.user_id;
		try {
			await removeMember(organizationId, member.user_id);
			members = members.filter((x) => x.user_id !== member.user_id);
			toast.success(t('organizations.members.removed', { name: member.name || member.email }));
		} catch (err) {
			toast.error(err instanceof Error ? err.message : t('organizations.members.remove_failed'));
		} finally {
			removingId = null;
			confirmMember = null;
		}
	}
</script>

<section class="space-y-3" aria-labelledby="members-heading">
	<div>
		<h3 id="members-heading" class="text-[15px] font-semibold">
			{t('organizations.members.title')}
			<span class="text-muted-foreground">({members.length})</span>
		</h3>
		<p class="mt-1 text-sm text-muted-foreground">{t('organizations.members.description')}</p>
	</div>
	{#if members.length === 0}
		<div
			class="flex flex-col items-start gap-3 rounded-2xl border border-dashed border-foreground/15 bg-card p-5">
			<div class="flex size-9 items-center justify-center lp-raised rounded-xl border bg-muted">
				<UsersIcon class="size-4 text-muted-foreground" />
			</div>
			<div>
				<p class="font-medium">{t('organizations.members.empty')}</p>
				<p class="mt-1 text-sm text-muted-foreground">{t('organizations.members.empty_hint')}</p>
			</div>
		</div>
	{:else}
		<ul class="lp-raised overflow-hidden rounded-2xl border bg-card">
			{#each members as member (member.user_id)}
				<li
					class="flex flex-col gap-3 p-4 not-last:border-b not-last:border-border/60 sm:flex-row sm:items-center">
					<Avatar.Root class="size-9 rounded-lg">
						<Avatar.Image
							src={avatarUrl(member.name || member.email)}
							alt={t('organizations.common.avatar_alt', { name: member.name || member.email })} />
						<Avatar.Fallback class="rounded-lg">
							{initials(member.name || member.email)}
						</Avatar.Fallback>
					</Avatar.Root>
					<div class="flex min-w-0 flex-1 flex-col">
						<span class="flex items-center gap-2 font-medium">
							{member.name || member.email}
							{#if member.disabled}
								<Badge variant="destructive">{t('organizations.members.disabled_badge')}</Badge>
							{/if}
							<!-- Null when the caller may not see it; only member managers are told. -->
							{#if member.two_factor_enabled === true}
								<Badge variant="outline" class="text-muted-foreground">
									{t('organizations.members.two_factor_on')}
								</Badge>
							{:else if member.two_factor_enabled === false}
								<Badge variant="outline" class="text-amber-600 dark:text-amber-400">
									{t('organizations.members.two_factor_off')}
								</Badge>
							{/if}
						</span>
						<span class="break-words text-sm text-muted-foreground">{member.email}</span>
					</div>
					{#if isSysadmin && member.user_id !== myUserId}
						<label
							class="flex min-h-11 items-center gap-2 text-sm text-muted-foreground sm:min-h-9">
							<!-- The getter keeps the switch on the saved state until confirmed. -->
							<Switch
								bind:checked={() => !member.disabled, () => (toggling = member)}
								aria-label={t('organizations.members.enabled_label', {
									name: member.name || member.email,
								})} />
							{member.disabled
								? t('organizations.members.disabled_state')
								: t('organizations.members.enabled_state')}
						</label>
					{/if}
					{#if canManageMembers}
						<NativeSelect
							value={String(member.role_id)}
							onchange={(e) => changeRole(member, e.currentTarget)}
							aria-label={t('organizations.members.base_role')}
							class="w-full sm:w-36">
							{#each BASE_ROLES as [id, name] (id)}
								<option value={String(id)}>{name}</option>
							{/each}
						</NativeSelect>
						<Button
							variant="ghost"
							size="sm"
							class="min-h-11 sm:min-h-9"
							onclick={() => (inspecting = member)}>
							<ShieldIcon class="size-4" />
							{t('organizations.members.permissions')}
						</Button>
					{:else}
						<Badge variant="secondary" class="w-fit sm:shrink-0">{member.role_name}</Badge>
					{/if}
					{#if canManageMembers}
						<Button
							variant="ghost"
							size="sm"
							class="min-h-11 text-destructive hover:text-destructive sm:min-h-9"
							disabled={removingId === member.user_id}
							onclick={() => (confirmMember = member)}>
							<Trash2Icon class="size-4" />
							{removingId === member.user_id
								? t('organizations.members.removing')
								: t('organizations.members.remove')}
						</Button>
					{/if}
				</li>
			{/each}
		</ul>
	{/if}
	{#if !canManageMembers}
		<p class="text-muted-foreground text-xs">
			{t('organizations.members.only_admins')}
		</p>
	{/if}
</section>

<AlertDialog.Root
	open={confirmMember !== null}
	onOpenChange={(o) => {
		if (!o) confirmMember = null;
	}}>
	<AlertDialog.Content>
		<AlertDialog.Header>
			<AlertDialog.Title>{t('organizations.members.remove_confirm')}</AlertDialog.Title>
			<AlertDialog.Description>
				{confirmMember
					? t('organizations.members.remove_description', {
							name: confirmMember.name || confirmMember.email,
						})
					: ''}
			</AlertDialog.Description>
		</AlertDialog.Header>
		<AlertDialog.Footer>
			<AlertDialog.Cancel>{t('organizations.common.cancel')}</AlertDialog.Cancel>
			<DeleteConfirmControl
				label={t('organizations.members.remove_label')}
				pendingLabel={t('organizations.members.remove_pending')}
				disabled={!confirmMember}
				onconfirm={async () => {
					if (confirmMember) await handleRemove(confirmMember);
				}} />
		</AlertDialog.Footer>
	</AlertDialog.Content>
</AlertDialog.Root>

<AlertDialog.Root
	open={toggling !== null}
	onOpenChange={(o) => {
		if (!o && !togglingBusy) toggling = null;
	}}>
	<AlertDialog.Content>
		<AlertDialog.Header>
			<AlertDialog.Title>
				{toggling?.disabled
					? t('organizations.members.enable_confirm')
					: t('organizations.members.disable_confirm')}
			</AlertDialog.Title>
			<AlertDialog.Description>
				{toggling
					? t(
							toggling.disabled
								? 'organizations.members.enable_description'
								: 'organizations.members.disable_description',
							{ name: toggling.name || toggling.email },
						)
					: ''}
			</AlertDialog.Description>
		</AlertDialog.Header>
		<AlertDialog.Footer>
			<AlertDialog.Cancel disabled={togglingBusy}
				>{t('organizations.common.cancel')}</AlertDialog.Cancel>
			<Button
				variant={toggling?.disabled ? 'default' : 'destructive'}
				disabled={togglingBusy}
				onclick={confirmToggle}>
				{toggling?.disabled
					? t('organizations.members.enable_action')
					: t('organizations.members.disable_action')}
			</Button>
		</AlertDialog.Footer>
	</AlertDialog.Content>
</AlertDialog.Root>

<EffectivePermissionsDialog {organizationId} bind:member={inspecting} />
