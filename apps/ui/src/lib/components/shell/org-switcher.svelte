<script lang="ts">
	import { goto } from '$app/navigation';
	import * as ContextMenu from '#lib/components/ui/context-menu/index.js';
	import * as Command from '#lib/components/ui/command/index.js';
	import * as Popover from '#lib/components/ui/popover/index.js';
	import ChevronsUpDownIcon from '@lucide/svelte/icons/chevrons-up-down';
	import PencilIcon from '@lucide/svelte/icons/pencil';
	import PlusIcon from '@lucide/svelte/icons/plus';
	import UsersIcon from '@lucide/svelte/icons/users';
	import type { OrganizationDto } from '#lib/clients/organization-client.js';
	import { setSelectedOrganizationId } from '#lib/utils/selected-organization.js';
	import OrganizationIcon from '#lib/components/organization-icon.svelte';
	import OrganizationEditDialog from '#lib/components/organizations/organization-edit-dialog.svelte';
	import { t } from '#lib/i18n/index.svelte.js';

	let {
		organizations = $bindable<OrganizationDto[]>([]),
		active,
	}: { organizations?: OrganizationDto[]; active: OrganizationDto | null } = $props();

	let open = $state(false);
	let query = $state('');
	let editDialogOpen = $state(false);
	let editingOrganization = $state<OrganizationDto | null>(null);

	// Your current organization leads, the rest follow alphabetically: a sysadmin
	// or cloud operator sees everyone's, and API order buries their own.
	const ordered = $derived(
		[...organizations].sort(
			(a, b) =>
				Number(b.id === active?.id) - Number(a.id === active?.id) || a.name.localeCompare(b.name),
		),
	);
	// The list filters as you type; Enter picks the highlighted (first) match.
	const matches = $derived.by(() => {
		const needle = query.trim().toLowerCase();
		return needle
			? ordered.filter((organization) => organization.name.toLowerCase().includes(needle))
			: ordered;
	});

	function selectOrganization(organization: OrganizationDto) {
		open = false;
		setSelectedOrganizationId(organization.id);
		// Keep the section (`?tab=`); other params (a server id) belong to the old org.
		const tab = new URLSearchParams(window.location.search).get('tab');
		window.location.assign(
			tab ? `${window.location.pathname}?tab=${encodeURIComponent(tab)}` : window.location.pathname,
		);
	}

	function applyUpdate(updated: OrganizationDto) {
		organizations = organizations.map((organization) =>
			organization.id === updated.id ? updated : organization,
		);
	}
</script>

{#snippet tile(organization: OrganizationDto)}
	<span
		class="{organization.icon_bg
			? ''
			: 'bg-primary text-primary-foreground'} lp-raised flex size-5.5 shrink-0 items-center justify-center overflow-hidden rounded-md">
		<OrganizationIcon icon={organization.icon} bg={organization.icon_bg} class="size-3.5" />
	</span>
{/snippet}

<Popover.Root
	bind:open
	onOpenChange={(value) => {
		if (value) query = '';
	}}>
	<Popover.Trigger
		class="flex h-8 min-w-0 items-center gap-2 rounded-lg px-2 text-sm font-medium outline-none transition-colors hover:bg-muted focus-visible:ring-3 focus-visible:ring-ring/30 data-[state=open]:bg-muted">
		{#if active}
			{@render tile(active)}
			<span class="truncate max-sm:hidden">{active.name}</span>
		{:else}
			<span class="truncate text-muted-foreground">{t('organizations.switcher.none')}</span>
		{/if}
		<ChevronsUpDownIcon class="size-3.5 shrink-0 text-muted-foreground" />
	</Popover.Trigger>
	<Popover.Content class="w-64 rounded-xl p-0" align="start" sideOffset={6}>
		<Command.Root shouldFilter={false}>
			<Command.Input bind:value={query} placeholder={t('organizations.switcher.search')} />
			<Command.List class="max-h-80">
				{#if matches.length === 0}
					<Command.Empty>{t('organizations.switcher.no_match')}</Command.Empty>
				{:else}
					<Command.Group heading={t('organizations.list.title')}>
						{#each matches as organization (organization.id)}
							<ContextMenu.Root>
								<ContextMenu.Trigger class="w-full">
									<Command.Item
										value={organization.id}
										onSelect={() => selectOrganization(organization)}
										class="gap-2 p-2">
										{@render tile(organization)}
										<span class="flex min-w-0 flex-1 flex-col">
											<span class="truncate">{organization.name}</span>
											<span class="text-xs text-muted-foreground">{organization.plan}</span>
										</span>
									</Command.Item>
								</ContextMenu.Trigger>
								<ContextMenu.Content>
									<ContextMenu.Item
										onclick={() => {
											open = false;
											editingOrganization = organization;
											editDialogOpen = true;
										}}>
										<PencilIcon class="mr-2 size-4" />
										{t('organizations.edit.title')}
									</ContextMenu.Item>
								</ContextMenu.Content>
							</ContextMenu.Root>
						{/each}
					</Command.Group>
				{/if}
				<Command.Separator />
				<Command.Group>
					<Command.Item
						value="__all"
						class="gap-2 p-2"
						onSelect={() => {
							open = false;
							void goto('/organizations');
						}}>
						<UsersIcon class="size-4" />
						<span class="text-muted-foreground">{t('organizations.switcher.all')}</span>
					</Command.Item>
					<Command.Item
						value="__add"
						class="gap-2 p-2"
						onSelect={() => {
							open = false;
							void goto('/organizations/add');
						}}>
						<PlusIcon class="size-4" />
						<span class="text-muted-foreground">{t('organizations.list.add')}</span>
					</Command.Item>
				</Command.Group>
			</Command.List>
		</Command.Root>
	</Popover.Content>
</Popover.Root>

<OrganizationEditDialog
	bind:open={editDialogOpen}
	organization={editingOrganization}
	onSaved={applyUpdate} />
