<script lang="ts">
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Command from '#lib/components/ui/command/index.js';
	import * as Popover from '#lib/components/ui/popover/index.js';
	import ServerOption from './server-option.svelte';
	import ChevronDownIcon from '@lucide/svelte/icons/chevron-down';
	import { cn } from '#lib/utils.js';

	/** A server row from either client shape: the summary carries the platform marks,
	 *  while a full `Server` keeps its distribution in `facts.os`. */
	export type ServerPickerServer = {
		id: string;
		name: string;
		os?: string | null;
		distribution?: string | null;
		ingress_mode?: string | null;
		kind?: string;
		facts?: { os: string | null } | null;
	};

	let {
		servers,
		value = $bindable(''),
		placeholder,
		searchPlaceholder,
		emptyText,
		detailFor = null,
		onSelect,
		disabled = false,
		triggerClass = '',
	}: {
		servers: ServerPickerServer[];
		value?: string;
		placeholder: string;
		searchPlaceholder: string;
		emptyText: string;
		/** Replaces the default `ingress_mode` line under each server. */
		detailFor?: ((server: ServerPickerServer) => string | null) | null;
		onSelect?: (value: string) => void;
		disabled?: boolean;
		triggerClass?: string;
	} = $props();

	let open = $state(false);

	function platform(server: ServerPickerServer) {
		if (server.os || server.distribution) {
			return { os: server.os ?? null, distribution: server.distribution ?? null };
		}
		return {
			os: null,
			distribution: server.kind === 'local' ? null : (server.facts?.os ?? null),
		};
	}

	const items = $derived(
		servers.map((server) => ({
			value: server.id,
			name: server.name,
			...platform(server),
			detail: detailFor ? detailFor(server) : (server.ingress_mode?.replace('_', ' ') ?? null),
		})),
	);
	const selected = $derived(items.find((item) => item.value === value) ?? null);

	function choose(next: (typeof items)[number]) {
		value = next.value;
		open = false;
		onSelect?.(next.value);
	}
</script>

<Popover.Root bind:open>
	<Popover.Trigger>
		{#snippet child({ props })}
			<Button
				{...props}
				variant="outline"
				class={cn('h-9 w-full justify-between px-3 font-normal', triggerClass)}
				role="combobox"
				aria-expanded={open}
				aria-label={placeholder}
				{disabled}>
				<ServerOption server={selected} detail={selected?.detail} {placeholder} class="flex-1" />
				<ChevronDownIcon class="size-4 shrink-0 opacity-60" />
			</Button>
		{/snippet}
	</Popover.Trigger>
	<Popover.Content class="w-72 p-0" align="start">
		<Command.Root>
			<Command.Input placeholder={searchPlaceholder} />
			<Command.List>
				<Command.Empty>{emptyText}</Command.Empty>
				{#each items as item (item.value)}
					<Command.Item
						value={[item.name, item.detail, item.distribution, item.os].filter(Boolean).join(' ')}
						data-checked={item.value === value}
						onSelect={() => choose(item)}>
						<ServerOption server={item} detail={item.detail} class="flex-1" />
					</Command.Item>
				{/each}
			</Command.List>
		</Command.Root>
	</Popover.Content>
</Popover.Root>
