<script lang="ts">
	import type { Snippet } from 'svelte';
	import { Button, type ButtonSize, type ButtonVariant } from '#lib/components/ui/button/index.js';
	import * as Command from '#lib/components/ui/command/index.js';
	import * as Popover from '#lib/components/ui/popover/index.js';
	import { cn } from '#lib/utils.js';
	import CheckIcon from '@lucide/svelte/icons/check';
	import ChevronDownIcon from '@lucide/svelte/icons/chevron-down';

	export type SearchableDropdownItem = {
		value: string;
		label: string;
		description?: string;
		/** Consumer-defined key rendered by the `icon` snippet. */
		icon?: string;
		/** Puts the item under a headed section. Items without one stay in a
		 *  single unheaded group, which is how every existing caller behaves. */
		group?: string;
		/** Visible and searchable, but not selectable. For an option the caller
		 *  wants the user to find and understand rather than never see. */
		disabled?: boolean;
		/** A short tag on the right of the row, e.g. `tools` or `endpoint`. */
		badge?: string;
	};

	let {
		items,
		selectedValue = $bindable<string | null>(null),
		label = 'Select an option',
		searchPlaceholder = 'Search…',
		emptyText = 'No options found.',
		disabled = false,
		onSelect,
		icon,
		variant = 'outline',
		size = 'default',
		triggerClass = '',
	}: {
		items: SearchableDropdownItem[];
		selectedValue?: string | null;
		label?: string;
		searchPlaceholder?: string;
		emptyText?: string;
		disabled?: boolean;
		onSelect?: (value: string) => void;
		icon?: Snippet<[string]>;
		variant?: ButtonVariant;
		size?: ButtonSize;
		triggerClass?: string;
	} = $props();

	let open = $state(false);
	let selectedItem = $derived(items.find((item) => item.value === selectedValue) ?? null);

	/** Groups in first-seen order, so the caller's ordering is the one shown. */
	let groups = $derived.by(() => {
		const byHeading = new Map<string, SearchableDropdownItem[]>();
		for (const item of items) {
			const heading = item.group ?? '';
			const existing = byHeading.get(heading);
			if (existing) existing.push(item);
			else byHeading.set(heading, [item]);
		}
		return [...byHeading];
	});

	function select(item: SearchableDropdownItem) {
		if (item.disabled) return;
		selectedValue = item.value;
		open = false;
		onSelect?.(item.value);
	}
</script>

<Popover.Root bind:open>
	<Popover.Trigger>
		{#snippet child({ props })}
			<Button
				{...props}
				{variant}
				{size}
				class={cn('min-w-52 justify-between', triggerClass)}
				role="combobox"
				aria-expanded={open}
				aria-label={label}
				{disabled}>
				{#if icon && selectedItem?.icon}{@render icon(selectedItem.icon)}{/if}
				<span class="truncate">{selectedItem?.label ?? label}</span>
				<ChevronDownIcon class="size-4 shrink-0 opacity-60" />
			</Button>
		{/snippet}
	</Popover.Trigger>
	<Popover.Content class="w-72 p-0" align="start">
		<Command.Root>
			<Command.Input placeholder={searchPlaceholder} />
			<Command.List>
				<Command.Empty>{emptyText}</Command.Empty>
				{#each groups as [heading, groupItems] (heading)}
					<Command.Group heading={heading || undefined}>
						{#each groupItems as item (item.value)}
							<Command.Item
								value={`${heading} ${item.label} ${item.description ?? ''}`}
								disabled={item.disabled}
								onSelect={() => select(item)}>
								{#if icon && item.icon}{@render icon(item.icon)}{/if}
								<div class="min-w-0 flex-1">
									<p class="truncate">{item.label}</p>
									{#if item.description}<p class="truncate text-xs text-muted-foreground">
											{item.description}
										</p>{/if}
								</div>
								{#if item.badge}
									<span
										class="text-muted-foreground shrink-0 rounded border px-1.5 py-0.5 font-mono text-[10px] uppercase"
										>{item.badge}</span>
								{/if}
								{#if item.value === selectedValue}<CheckIcon class="size-4" />{/if}
							</Command.Item>
						{/each}
					</Command.Group>
				{/each}
			</Command.List>
		</Command.Root>
	</Popover.Content>
</Popover.Root>
