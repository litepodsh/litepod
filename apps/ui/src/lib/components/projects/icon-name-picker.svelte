<script lang="ts">
	import type { Snippet } from 'svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Command from '#lib/components/ui/command/index.js';
	import * as Popover from '#lib/components/ui/popover/index.js';
	import OrganizationIcon from '#lib/components/organization-icon.svelte';
	import { t } from '#lib/i18n/index.svelte.js';
	import { isNamedIcon, ORGANIZATION_ICON_MAP } from '#lib/utils/organization-icons.js';
	import ChevronDownIcon from '@lucide/svelte/icons/chevron-down';
	import XIcon from '@lucide/svelte/icons/x';

	let {
		value,
		disabled = false,
		onpick,
		children,
	}: {
		/** The current icon: a name, an image URL or blank. */
		value: string;
		disabled?: boolean;
		/** A name from the list, or blank to remove the icon. */
		onpick: (name: string) => void;
		/** What the trigger shows for the current icon. */
		children?: Snippet;
	} = $props();

	const names = Object.keys(ORGANIZATION_ICON_MAP);
	const label = (name: string) => t(`projects.projects.icon_names.${name}`);

	let open = $state(false);
	let highlighted = $state('');

	function pick(name: string) {
		onpick(name);
		open = false;
	}
</script>

<Popover.Root
	bind:open
	onOpenChange={(next) => {
		if (next) highlighted = isNamedIcon(value) ? value : '';
	}}>
	<Popover.Trigger {disabled}>
		{#snippet child({ props })}
			<Button
				{...props}
				type="button"
				variant="outline"
				class="h-9 shrink-0 gap-1 px-2"
				aria-label={t('projects.projects.icon_pick')}>
				<span class="grid size-5 place-items-center overflow-hidden">{@render children?.()}</span>
				<ChevronDownIcon class="size-3.5 text-muted-foreground" />
			</Button>
		{/snippet}
	</Popover.Trigger>
	<Popover.Content class="w-[17rem] p-0" align="start" sideOffset={6}>
		<Command.Root columns={6} bind:value={highlighted}>
			<Command.Input placeholder={t('projects.projects.icon_search')} />
			<!-- A fixed height: fewer results must not flip the panel to the other side. -->
			<Command.List class="h-[19.5rem] max-h-none p-1">
				<Command.Empty>{t('projects.projects.icon_empty')}</Command.Empty>
				<div class="grid grid-cols-6 gap-1">
					{#each names as name (name)}
						<Command.Item
							value={name}
							keywords={[label(name)]}
							aria-label={label(name)}
							title={label(name)}
							data-checked={value === name}
							class="aspect-square justify-center p-0 data-[checked=true]:bg-primary/10 data-[checked=true]:text-primary data-[checked=true]:ring-1 data-[checked=true]:ring-primary/50 [&>.cn-command-item-indicator]:hidden"
							onSelect={() => pick(name)}>
							<OrganizationIcon icon={name} class="size-4" />
						</Command.Item>
					{/each}
				</div>
			</Command.List>
			<div class="flex h-8 items-center justify-between gap-2 border-t px-2 pt-1 text-xs">
				<span class="truncate text-muted-foreground">
					{isNamedIcon(highlighted) ? label(highlighted) : ''}
				</span>
				{#if value}
					<Button
						type="button"
						variant="ghost"
						size="xs"
						class="shrink-0"
						onclick={() => pick('')}>
						<XIcon class="size-3" />
						{t('projects.projects.icon_remove')}
					</Button>
				{/if}
			</div>
		</Command.Root>
	</Popover.Content>
</Popover.Root>
