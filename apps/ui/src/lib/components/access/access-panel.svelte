<script lang="ts">
	import { onMount } from 'svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import { NativeSelect } from '#lib/components/ui/native-select/index.js';
	import SkeletonRows from '#lib/components/custom/skeleton-rows.svelte';
	import Trash2Icon from '@lucide/svelte/icons/trash-2';
	import {
		createAssignment,
		deleteAssignment,
		inheritedAssignments,
		listAssignments,
		listGroups,
		listRoles,
		type Assignment,
		type Group,
		type Role,
		type ScopeType,
	} from '#lib/clients/access-client.js';
	import { listMembers, type OrganizationMemberDto } from '#lib/clients/organization-client.js';
	import { allowed } from '#lib/state/permissions.svelte.js';
	import { toast } from '#lib/utils/toast.js';
	import { t } from '#lib/i18n/index.svelte.js';

	/** Who holds which role at one scope, plus what reaches it from above. */
	let {
		organizationId,
		scopeType,
		scopeId,
		projectId,
		knownMembers,
	}: {
		organizationId: string;
		scopeType: ScopeType;
		scopeId: string;
		/** Owning project, for an environment scope. */
		projectId?: string;
		/** Already loaded by the page; fetched here when not given. */
		knownMembers?: OrganizationMemberDto[];
	} = $props();

	let direct = $state<Assignment[]>([]);
	let inherited = $state<Assignment[]>([]);
	let roles = $state<Role[]>([]);
	let groups = $state<Group[]>([]);
	let members = $state<OrganizationMemberDto[]>([]);
	let loading = $state(true);
	let busy = $state(false);
	let principal = $state('');
	let roleId = $state('2');

	const canManage = $derived(allowed('org.members.manage'));
	// Member (4) is a base role, not something to assign.
	const assignable = $derived(roles.filter((r) => r.id !== '4'));

	onMount(async () => {
		try {
			const [atScope, all, r, g, m] = await Promise.all([
				listAssignments(organizationId, { scopeType, scopeId }),
				scopeType === 'org' ? Promise.resolve([]) : listAssignments(organizationId),
				listRoles(organizationId),
				listGroups(organizationId),
				knownMembers ? Promise.resolve(knownMembers) : listMembers(organizationId),
			]);
			direct = atScope;
			inherited = inheritedAssignments(all, { scopeType, scopeId, projectId });
			[roles, groups, members] = [r, g, m];
		} catch (err) {
			toast.error(err instanceof Error ? err.message : t('organizations.access.load_failed'));
		} finally {
			loading = false;
		}
	});

	async function add(event: SubmitEvent) {
		event.preventDefault();
		const [principal_type, principal_id] = principal.split(':') as ['user' | 'group', string];
		busy = true;
		try {
			const created = await createAssignment(organizationId, {
				principal_type,
				principal_id,
				role_id: roleId,
				scope_type: scopeType,
				scope_id: scopeId,
			});
			direct = [...direct, created];
			principal = '';
		} catch (err) {
			toast.error(err instanceof Error ? err.message : t('organizations.access.save_failed'));
		} finally {
			busy = false;
		}
	}

	async function remove(assignment: Assignment) {
		try {
			await deleteAssignment(organizationId, assignment.id);
			direct = direct.filter((a) => a.id !== assignment.id);
		} catch (err) {
			toast.error(err instanceof Error ? err.message : t('organizations.access.save_failed'));
		}
	}
</script>

{#snippet row(assignment: Assignment, origin: string | null)}
	<li
		class="flex flex-col gap-2 p-3 not-last:border-b not-last:border-border/60 sm:flex-row sm:items-center {origin
			? 'text-muted-foreground'
			: ''}">
		<span class="flex min-w-0 flex-1 items-center gap-2">
			<span class="truncate font-medium">{assignment.principal_name}</span>
			{#if assignment.principal_type === 'group'}<Badge variant="outline"
					>{t('organizations.access.group')}</Badge
				>{/if}
		</span>
		<Badge variant="secondary" class="w-fit">{assignment.role_name}</Badge>
		{#if origin}
			<span class="text-xs">{origin}</span>
		{:else if canManage}
			<Button
				variant="ghost"
				size="sm"
				class="text-destructive hover:text-destructive"
				aria-label={t('organizations.access.remove')}
				onclick={() => remove(assignment)}><Trash2Icon class="size-4" /></Button>
		{/if}
	</li>
{/snippet}

<section class="space-y-3" aria-labelledby="access-heading">
	<div>
		<h3 id="access-heading" class="text-[15px] font-semibold">
			{t('organizations.access.access')}
		</h3>
		<p class="mt-1 text-sm text-muted-foreground">
			{t(`organizations.access.access_hint_${scopeType}`)}
		</p>
	</div>

	{#if canManage && !loading}
		<form class="flex flex-wrap gap-2" onsubmit={add}>
			<NativeSelect
				bind:value={principal}
				class="w-full sm:w-64"
				aria-label={t('organizations.access.who')}>
				<option value="" disabled>{t('organizations.access.who')}</option>
				<optgroup label={t('organizations.access.people')}>
					{#each members as member (member.user_id)}
						<option value={`user:${member.user_id}`}>{member.name || member.email}</option>
					{/each}
				</optgroup>
				{#if groups.length}
					<optgroup label={t('organizations.access.groups')}>
						{#each groups as group (group.id)}
							<option value={`group:${group.id}`}>{group.name}</option>
						{/each}
					</optgroup>
				{/if}
			</NativeSelect>
			<NativeSelect
				bind:value={roleId}
				class="w-full sm:w-48"
				aria-label={t('organizations.access.role')}>
				{#each assignable as role (role.id)}
					<option value={role.id}>{role.name}</option>
				{/each}
			</NativeSelect>
			<Button type="submit" disabled={busy || !principal}>{t('organizations.access.grant')}</Button>
		</form>
	{/if}

	{#if loading}
		<SkeletonRows rows={3} height="h-12" label={t('organizations.access.loading')} />
	{:else if direct.length === 0 && inherited.length === 0}
		<p class="text-sm text-muted-foreground">{t('organizations.access.no_assignments')}</p>
	{:else}
		<ul class="lp-raised overflow-hidden rounded-2xl border bg-card">
			{#each direct as assignment (assignment.id)}
				{@render row(assignment, null)}
			{/each}
			{#each inherited as assignment (assignment.id)}
				{@render row(
					assignment,
					t('organizations.access.inherited_from', { name: assignment.scope_name }),
				)}
			{/each}
		</ul>
	{/if}
</section>
