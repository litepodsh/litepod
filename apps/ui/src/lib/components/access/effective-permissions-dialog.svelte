<script lang="ts">
	import * as Dialog from '#lib/components/ui/dialog/index.js';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import SkeletonRows from '#lib/components/custom/skeleton-rows.svelte';
	import {
		getEffectivePermissions,
		type EffectivePermissions,
	} from '#lib/clients/access-client.js';
	import type { OrganizationMemberDto } from '#lib/clients/organization-client.js';
	import { toast } from '#lib/utils/toast.js';
	import { t } from '#lib/i18n/index.svelte.js';

	let {
		organizationId,
		member = $bindable<OrganizationMemberDto | null>(null),
	}: { organizationId: string; member?: OrganizationMemberDto | null } = $props();

	let effective = $state<EffectivePermissions | null>(null);

	$effect(() => {
		const userId = member?.user_id;
		effective = null;
		if (!userId) return;
		getEffectivePermissions(organizationId, userId)
			.then((result) => {
				if (member?.user_id === userId) effective = result;
			})
			.catch((err) =>
				toast.error(err instanceof Error ? err.message : t('organizations.access.load_failed')),
			);
	});
</script>

<Dialog.Root
	open={member !== null}
	onOpenChange={(o) => {
		if (!o) member = null;
	}}>
	<Dialog.Content class="flex flex-col gap-0 overflow-y-hidden p-0 sm:max-w-2xl">
		<Dialog.Header class="border-b p-6 pr-14">
			<Dialog.Title>{member?.name || member?.email}</Dialog.Title>
			<Dialog.Description>{t('organizations.access.effective_hint')}</Dialog.Description>
		</Dialog.Header>
		<div class="min-h-0 flex-1 overflow-y-auto p-6">
			{#if !effective}
				<SkeletonRows rows={3} height="h-10" label={t('organizations.access.loading')} />
			{:else}
				<div class="grid gap-5">
					<div class="grid gap-2">
						<h4 class="text-sm font-medium">{t('organizations.access.grants')}</h4>
						<ul class="overflow-hidden rounded-xl text-sm ring-1 ring-border/70">
							<li class="flex flex-wrap items-center gap-2 p-3">
								<Badge variant="secondary">{effective.base_role_name}</Badge>
								<span class="text-muted-foreground">{t('organizations.access.base_role')}</span>
							</li>
							{#each effective.assignments as assignment (assignment.id)}
								<li class="flex flex-wrap items-center gap-2 border-t border-border/60 p-3">
									<Badge variant="secondary">{assignment.role_name}</Badge>
									<span>{assignment.scope_name}</span>
									<span class="text-muted-foreground">
										{assignment.via_group
											? t('organizations.access.via_group', { name: assignment.via_group })
											: t('organizations.access.direct')}
									</span>
								</li>
							{/each}
						</ul>
					</div>
					<div class="grid gap-2">
						<h4 class="text-sm font-medium">{t('organizations.access.org_wide')}</h4>
						<div class="flex flex-wrap gap-1">
							{#each effective.permissions.org as permission (permission)}
								<Badge variant="outline" class="font-mono text-[11px]">{permission}</Badge>
							{:else}
								<span class="text-sm text-muted-foreground">—</span>
							{/each}
						</div>
					</div>
				</div>
			{/if}
		</div>
	</Dialog.Content>
</Dialog.Root>
