<script lang="ts">
	import * as Dialog from '#lib/components/ui/dialog/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Label } from '#lib/components/ui/label/index.js';
	import { Textarea } from '#lib/components/ui/textarea/index.js';
	import OsIcon from '#lib/components/os-icon.svelte';
	import type { ServerSummaryDto } from '#lib/clients/project-client.js';
	import {
		createServerGroup,
		replaceServerGroupServers,
		updateServerGroup,
		type ServerGroup,
	} from '#lib/clients/server-group-client.js';
	import { t } from '#lib/i18n/index.svelte.js';
	import { osLabel } from '#lib/utils/os-label.js';
	import { toast } from '#lib/utils/toast.js';
	import { fade } from 'svelte/transition';
	import SearchIcon from '@lucide/svelte/icons/search';
	import PlusIcon from '@lucide/svelte/icons/plus';
	import XIcon from '@lucide/svelte/icons/x';
	import GripVerticalIcon from '@lucide/svelte/icons/grip-vertical';
	import ServerIcon from '@lucide/svelte/icons/server';
	import ArrowDownToLineIcon from '@lucide/svelte/icons/arrow-down-to-line';

	let {
		open = $bindable(false),
		organizationId,
		group,
		servers = [],
		preselected = [],
		onSaved,
	}: {
		open?: boolean;
		organizationId: string;
		/** The group being edited; `null` creates a new one. */
		group: ServerGroup | null;
		/** Ready remote servers eligible for membership. */
		servers?: ServerSummaryDto[];
		/** Server ids a new group starts with, e.g. the cards selected on the Servers page. */
		preselected?: string[];
		onSaved?: (group: ServerGroup) => void;
	} = $props();

	type Zone = 'available' | 'members';
	const DRAG_TYPE = 'application/x-litepod-server';

	let name = $state('');
	let description = $state('');
	let selected = $state<string[]>([]);
	let query = $state('');
	let saving = $state(false);
	let dragging = $state<{ id: string; from: Zone } | null>(null);
	let over = $state<Zone | null>(null);

	// Reseed on every open, so a cancelled edit does not linger in the inputs.
	$effect(() => {
		if (!open) return;
		name = group?.name ?? '';
		description = group?.description ?? '';
		selected = group?.member_servers.map((server) => server.id) ?? [...preselected];
		query = '';
	});

	const byId = $derived(new Map(servers.map((server) => [server.id, server])));
	// Members keep the order they were added in; ids for servers that are no longer eligible drop out.
	const members = $derived(
		selected.map((id) => byId.get(id)).filter((s): s is ServerSummaryDto => !!s),
	);
	const unselected = $derived(servers.filter((server) => !selected.includes(server.id)));
	const available = $derived.by(() => {
		const needle = query.trim().toLowerCase();
		if (!needle) return unselected;
		return unselected.filter((server) =>
			[server.name, server.distribution, server.os, server.os_version]
				.filter(Boolean)
				.some((field) => field!.toLowerCase().includes(needle)),
		);
	});

	function add(serverId: string) {
		if (!selected.includes(serverId)) selected = [...selected, serverId];
	}

	function remove(serverId: string) {
		selected = selected.filter((id) => id !== serverId);
	}

	function addVisible() {
		selected = [...selected, ...available.map((server) => server.id)];
	}

	function gib(bytes: number | null) {
		if (!bytes) return null;
		const value = bytes / 1024 ** 3;
		return `${value >= 10 ? Math.round(value) : value.toFixed(1)} GiB`;
	}

	function meta(server: ServerSummaryDto) {
		return [
			osLabel(server.distribution ?? server.os, server.os_version),
			server.capacity.cpu_cores
				? t('servers.card.vcpu', { count: server.capacity.cpu_cores })
				: null,
			gib(server.capacity.memory_bytes),
		]
			.filter(Boolean)
			.join(' · ');
	}

	function onDragStart(event: DragEvent, id: string, from: Zone) {
		if (!event.dataTransfer) return;
		event.dataTransfer.effectAllowed = 'move';
		event.dataTransfer.setData(DRAG_TYPE, id);
		event.dataTransfer.setData('text/plain', id);
		dragging = { id, from };
	}

	function onDragEnd() {
		dragging = null;
		over = null;
	}

	function onDragOver(event: DragEvent, zone: Zone) {
		// Only accept our own rows, and only into the opposite list.
		if (!dragging || dragging.from === zone) return;
		event.preventDefault();
		if (event.dataTransfer) event.dataTransfer.dropEffect = 'move';
		over = zone;
	}

	function onDragLeave(event: DragEvent, zone: Zone) {
		const target = event.currentTarget as HTMLElement;
		if (target.contains(event.relatedTarget as Node | null)) return;
		if (over === zone) over = null;
	}

	function onDrop(event: DragEvent, zone: Zone) {
		event.preventDefault();
		const id = event.dataTransfer?.getData(DRAG_TYPE) || dragging?.id;
		if (id && byId.has(id)) {
			if (zone === 'members') add(id);
			else remove(id);
		}
		onDragEnd();
	}

	async function save() {
		const trimmed = name.trim();
		if (!trimmed) {
			toast.error(t('organizations.server_groups.name_required'));
			return;
		}
		if (trimmed.length > 100) {
			toast.error(t('organizations.server_groups.name_invalid'));
			return;
		}
		saving = true;
		try {
			let saved: ServerGroup;
			if (group) {
				saved = await updateServerGroup(organizationId, group.id, {
					name: trimmed,
					description: description.trim(),
				});
			} else {
				saved = await createServerGroup(organizationId, {
					name: trimmed,
					description: description.trim(),
				});
			}
			saved = await replaceServerGroupServers(
				organizationId,
				saved.id,
				members.map((s) => s.id),
			);
			onSaved?.(saved);
			open = false;
		} catch (err) {
			toast.error(
				err instanceof Error ? err.message : t('organizations.server_groups.save_failed'),
			);
		} finally {
			saving = false;
		}
	}
</script>

{#snippet row(server: ServerSummaryDto, zone: Zone)}
	{@const isDragging = dragging?.id === server.id}
	<li
		in:fade={{ duration: 120 }}
		draggable="true"
		ondragstart={(event) => onDragStart(event, server.id, zone)}
		ondragend={onDragEnd}
		ondblclick={() => (zone === 'available' ? add(server.id) : remove(server.id))}
		class="group/row flex cursor-grab items-center gap-2.5 rounded-xl border bg-background px-2.5 py-2 shadow-xs transition-[opacity,box-shadow,border-color] select-none hover:border-foreground/20 hover:shadow-sm active:cursor-grabbing {isDragging
			? 'opacity-40'
			: ''}">
		<GripVerticalIcon
			class="size-3.5 shrink-0 text-muted-foreground/50 transition-colors group-hover/row:text-muted-foreground" />
		<span
			class="relative flex size-8 shrink-0 items-center justify-center rounded-lg bg-muted text-muted-foreground">
			<OsIcon os={server.os} distribution={server.distribution} class="size-4" />
			<span
				class="absolute -right-0.5 -bottom-0.5 size-2.5 rounded-full ring-2 ring-background {server.state ===
				'ready'
					? 'bg-emerald-500'
					: server.state === 'degraded'
						? 'bg-amber-500'
						: 'bg-muted-foreground/50'}"></span>
		</span>
		<span class="min-w-0 flex-1">
			<span class="block truncate text-sm font-medium">{server.name}</span>
			{#if meta(server)}
				<span class="block truncate text-xs text-muted-foreground">{meta(server)}</span>
			{/if}
		</span>
		{#if zone === 'available'}
			<Button
				type="button"
				variant="ghost"
				size="icon-sm"
				class="shrink-0 text-muted-foreground hover:text-foreground"
				title={t('organizations.server_groups.add')}
				aria-label={t('organizations.server_groups.add')}
				onclick={() => add(server.id)}>
				<PlusIcon />
			</Button>
		{:else}
			<Button
				type="button"
				variant="ghost"
				size="icon-sm"
				class="shrink-0 text-muted-foreground hover:text-destructive"
				title={t('organizations.server_groups.remove_member')}
				aria-label={t('organizations.server_groups.remove_member')}
				onclick={() => remove(server.id)}>
				<XIcon />
			</Button>
		{/if}
	</li>
{/snippet}

<Dialog.Root bind:open>
	<Dialog.Content class="sm:max-w-3xl">
		<Dialog.Header>
			<Dialog.Title
				>{group
					? t('organizations.server_groups.edit')
					: t('organizations.server_groups.create')}</Dialog.Title>
			<Dialog.Description>{t('organizations.server_groups.description')}</Dialog.Description>
		</Dialog.Header>
		<form
			class="min-w-0"
			onsubmit={(event) => {
				event.preventDefault();
				save();
			}}>
			<div class="grid gap-5 pb-6">
				<div class="grid gap-4 sm:grid-cols-[minmax(0,1fr)_minmax(0,1.4fr)]">
					<div class="grid content-start gap-2">
						<Label for="group-name">{t('organizations.server_groups.name')}</Label>
						<Input id="group-name" bind:value={name} maxlength={100} autofocus />
					</div>
					<div class="grid content-start gap-2">
						<Label for="group-description"
							>{t('organizations.server_groups.description_label')}</Label>
						<Textarea
							id="group-description"
							bind:value={description}
							placeholder={t('organizations.server_groups.description_placeholder')}
							maxlength={2000}
							rows={1}
							class="min-h-9" />
					</div>
				</div>

				{#if servers.length === 0}
					<div
						class="flex flex-col items-center gap-2 rounded-2xl border border-dashed px-6 py-10 text-center">
						<ServerIcon class="size-6 text-muted-foreground" />
						<p class="max-w-sm text-sm text-muted-foreground">
							{t('organizations.server_groups.no_remote_servers')}
						</p>
					</div>
				{:else}
					<div class="grid gap-2">
						<p class="text-xs text-muted-foreground">
							{t('organizations.server_groups.members_hint')}
						</p>
						<div class="grid gap-3 sm:grid-cols-2">
							<!-- Available -->
							<section
								aria-label={t('organizations.server_groups.available')}
								class="flex min-h-0 flex-col rounded-2xl border bg-muted/30 p-2 transition-colors {over ===
								'available'
									? 'border-destructive/50 bg-destructive/5'
									: ''}"
								ondragover={(event) => onDragOver(event, 'available')}
								ondragleave={(event) => onDragLeave(event, 'available')}
								ondrop={(event) => onDrop(event, 'available')}>
								<header class="flex items-center justify-between gap-2 px-1.5 pt-1 pb-2">
									<span class="flex items-center gap-2 text-sm font-medium">
										{t('organizations.server_groups.available')}
										<span
											class="rounded-full bg-muted px-1.5 py-0.5 text-[11px] font-medium text-muted-foreground tabular-nums"
											>{unselected.length}</span>
									</span>
									<Button
										type="button"
										variant="ghost"
										size="xs"
										disabled={available.length === 0}
										onclick={addVisible}>
										{t('organizations.server_groups.add_all')}
									</Button>
								</header>
								<div class="relative mb-2">
									<SearchIcon
										class="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-muted-foreground" />
									<Input
										type="search"
										bind:value={query}
										placeholder={t('organizations.server_groups.search_placeholder')}
										class="bg-background pl-9"
										onkeydown={(event) => {
											// Enter in the search box adds the single match instead of submitting.
											if (event.key !== 'Enter') return;
											event.preventDefault();
											if (available.length === 1) {
												add(available[0].id);
												query = '';
											}
										}} />
								</div>
								<ul class="h-64 space-y-1.5 overflow-y-auto p-0.5">
									{#each available as server (server.id)}
										{@render row(server, 'available')}
									{:else}
										<li
											class="flex h-full items-center justify-center px-4 text-center text-xs text-muted-foreground">
											{unselected.length === 0
												? t('organizations.server_groups.all_added')
												: t('organizations.server_groups.no_matches', { query: query.trim() })}
										</li>
									{/each}
								</ul>
							</section>

							<!-- Members -->
							<section
								aria-label={t('organizations.server_groups.members')}
								class="flex min-h-0 flex-col rounded-2xl border p-2 transition-colors {over ===
								'members'
									? 'border-primary/60 bg-primary/5 ring-4 ring-primary/10'
									: dragging?.from === 'available'
										? 'border-dashed border-primary/40'
										: ''}"
								ondragover={(event) => onDragOver(event, 'members')}
								ondragleave={(event) => onDragLeave(event, 'members')}
								ondrop={(event) => onDrop(event, 'members')}>
								<header class="flex items-center justify-between gap-2 px-1.5 pt-1 pb-2">
									<span class="flex items-center gap-2 text-sm font-medium">
										{t('organizations.server_groups.members')}
										<span
											class="rounded-full bg-primary/10 px-1.5 py-0.5 text-[11px] font-medium text-primary tabular-nums"
											>{members.length}</span>
									</span>
									<Button
										type="button"
										variant="ghost"
										size="xs"
										disabled={members.length === 0}
										onclick={() => (selected = [])}>
										{t('organizations.server_groups.clear_all')}
									</Button>
								</header>
								<ul class="h-[18.75rem] space-y-1.5 overflow-y-auto p-0.5">
									{#each members as server (server.id)}
										{@render row(server, 'members')}
									{:else}
										<li
											class="flex h-full flex-col items-center justify-center gap-2 rounded-xl border-2 border-dashed px-4 text-center transition-colors {over ===
											'members'
												? 'border-primary/50 text-primary'
												: 'border-muted text-muted-foreground'}">
											<ArrowDownToLineIcon
												class="size-5 {over === 'members' ? 'animate-bounce' : ''}" />
											<span class="text-sm font-medium"
												>{t('organizations.server_groups.drop_here')}</span>
											<span class="text-xs text-muted-foreground"
												>{t('organizations.server_groups.drop_here_hint')}</span>
										</li>
									{/each}
								</ul>
							</section>
						</div>
						<p class="text-center text-[11px] text-muted-foreground/80">
							{t('organizations.server_groups.drag_hint')}
						</p>
					</div>
				{/if}
			</div>
			<Dialog.Footer>
				<Button type="button" variant="outline" onclick={() => (open = false)}
					>{t('organizations.common.cancel')}</Button>
				<Button type="submit" disabled={saving}>
					{saving ? t('organizations.common.saving') : t('organizations.common.save')}
				</Button>
			</Dialog.Footer>
		</form>
	</Dialog.Content>
</Dialog.Root>
