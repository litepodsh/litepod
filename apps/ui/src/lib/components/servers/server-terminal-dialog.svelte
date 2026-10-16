<script lang="ts">
	import PlusIcon from '@lucide/svelte/icons/plus';
	import ServerIcon from '@lucide/svelte/icons/server';
	import XIcon from '@lucide/svelte/icons/x';
	import HistoryIcon from '@lucide/svelte/icons/history';
	import { listServerTerminalSessions } from '#lib/clients/terminal-client.js';
	import TerminalSessionHistory from '#lib/components/projects/terminal-session-history.svelte';
	import * as Dialog from '#lib/components/ui/dialog/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import AnimatedTabs from '#lib/components/custom/animated-tabs/animated-tabs.svelte';
	import type { Server } from '#lib/clients/server-client.js';
	import ServerTerminalTab from './server-terminal-tab.svelte';
	import { t } from '#lib/i18n/index.svelte.js';

	let {
		open = $bindable(false),
		organizationId,
		server = null,
		onOpenChange = () => {},
	}: {
		open?: boolean;
		organizationId: string;
		server?: Server | null;
		onOpenChange?: (open: boolean) => void;
	} = $props();
	type Tab = { id: string; label: string };
	let tabs = $state<Tab[]>([]);
	let activeTabId = $state('');
	let showHistory = $state(false);
	let lastServerId = '';
	let nextTabNumber = 1;
	function addTab() {
		const number = nextTabNumber++;
		const id = `shell-${number}`;
		tabs = [...tabs, { id, label: t('organizations.servers.shell_number', { number }) }];
		activeTabId = id;
		showHistory = false;
	}
	function close() {
		open = false;
		onOpenChange(false);
	}
	function closeTab(id: string) {
		const index = tabs.findIndex((tab) => tab.id === id);
		tabs = tabs.filter((tab) => tab.id !== id);
		if (activeTabId === id) activeTabId = tabs[Math.max(0, index - 1)]?.id ?? '';
		if (!tabs.length) close();
	}
	$effect(() => {
		if (open && server && server.id !== lastServerId) {
			lastServerId = server.id;
			tabs = [];
			activeTabId = '';
			nextTabNumber = 1;
			addTab();
		}
		if (!open) {
			tabs = [];
			activeTabId = '';
			lastServerId = '';
			showHistory = false;
		}
	});
</script>

<Dialog.Root
	{open}
	onOpenChange={(next) => {
		if (!next) close();
		else open = next;
	}}>
	<Dialog.Content
		class="!flex h-[min(46rem,calc(100dvh-1rem))] max-w-[calc(100%-1rem)] !flex-col gap-0 overflow-hidden p-0 sm:max-w-5xl">
		<Dialog.Header class="border-b bg-muted/30 px-5 py-4 pr-14"
			><div class="flex min-w-0 items-center gap-3">
				<span
					class="grid size-9 shrink-0 place-items-center rounded-xl border border-success/20 bg-success/10 text-success"
					><ServerIcon class="size-4" /></span>
				<div class="min-w-0">
					<Dialog.Title class="truncate"
						>{server?.name ?? t('organizations.servers.remote_console')}</Dialog.Title
					><Dialog.Description class="truncate font-mono text-xs"
						>{server?.ssh_user}@{server?.ssh_host}:{server?.ssh_port}</Dialog.Description>
				</div>
			</div></Dialog.Header>
		<div class="flex items-center gap-2 overflow-hidden border-b bg-muted/20 px-3 py-2">
			{#if activeTabId}
				{#key tabs.map((tab) => tab.id).join(':')}
					<AnimatedTabs
						items={tabs.map((tab) => ({ value: tab.id, label: tab.label }))}
						bind:value={activeTabId}
						class="min-w-0 flex-1" />
				{/key}
			{:else}
				<div class="h-10 flex-1" aria-hidden="true"></div>
			{/if}
			<Button
				size="sm"
				variant={showHistory ? 'secondary' : 'ghost'}
				class="shrink-0"
				aria-pressed={showHistory}
				onclick={() => (showHistory = !showHistory)}
				><HistoryIcon class="size-4" /><span class="max-sm:sr-only"
					>{t('projects.application_terminal.recent_sessions')}</span
				></Button>
			<Button size="sm" variant="outline" class="shrink-0" onclick={addTab}
				><PlusIcon class="size-4" />{t('organizations.servers.new_tab')}</Button>
			<Button
				size="icon-sm"
				variant="ghost"
				class="shrink-0"
				onclick={() => activeTabId && closeTab(activeTabId)}
				aria-label={t('organizations.servers.close_terminal_tab')}><XIcon class="size-4" /></Button>
		</div>
		<div class="relative min-h-0 flex-1">
			{#each tabs as tab (tab.id)}<ServerTerminalTab
					active={tab.id === activeTabId}
					{organizationId}
					serverId={server?.id ?? ''} />{/each}
			{#if showHistory && server}
				<!-- Overlay, not a swap: open shells stay connected underneath. -->
				<div class="absolute inset-0 z-10 overflow-y-auto bg-background p-3">
					<TerminalSessionHistory
						hostLabel={server.name}
						load={(page) => listServerTerminalSessions(organizationId, server.id, page)} />
				</div>
			{/if}
		</div>
	</Dialog.Content>
</Dialog.Root>
