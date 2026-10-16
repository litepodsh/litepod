<script lang="ts">
	import LoaderCircleIcon from '@lucide/svelte/icons/loader-circle';
	import MemoryStickIcon from '@lucide/svelte/icons/memory-stick';
	import * as Alert from '#lib/components/ui/alert/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import CopyButton from '#lib/components/custom/copy-button.svelte';
	import { createServerSwap, type Server } from '#lib/clients/server-client.js';
	import { toast } from '#lib/utils/toast.js';
	import { t } from '#lib/i18n/index.svelte.js';

	let {
		organizationId,
		server,
		onCreated,
	}: {
		organizationId: string;
		server: Server;
		/** Re-probe the server so `swap_recommended` clears. */
		onCreated: () => void;
	} = $props();

	// Mirrors the CREATE_SWAP script in api/src/servers/setup.rs.
	const MANUAL_COMMANDS = [
		'sudo dd if=/dev/zero of=/swapfile bs=1M count=4096',
		'sudo chmod 600 /swapfile',
		'sudo mkswap /swapfile',
		'sudo swapon /swapfile',
		"echo '/swapfile none swap sw 0 0' | sudo tee -a /etc/fstab",
	].join('\n');

	let creating = $state(false);
	let showManual = $state(false);
	let failure = $state('');

	async function create() {
		creating = true;
		failure = '';
		try {
			const result = await createServerSwap(organizationId, server.id);
			if (result.success) {
				toast.success(t('servers.swap.created_toast', { name: server.name }));
				onCreated();
			} else {
				failure = (result.stderr || result.stdout).trim() || t('servers.swap.failed');
				showManual = true;
			}
		} catch (caught) {
			failure = caught instanceof Error ? caught.message : t('servers.swap.failed');
		} finally {
			creating = false;
		}
	}
</script>

<Alert.Root>
	<MemoryStickIcon />
	<Alert.Title>{t('servers.swap.title')}</Alert.Title>
	<Alert.Description class="space-y-3">
		<p>{t('servers.swap.explanation')}</p>
		<div class="flex flex-wrap gap-2">
			<Button size="sm" disabled={creating} onclick={() => void create()}>
				{#if creating}<LoaderCircleIcon class="size-4 animate-spin" />{/if}
				{creating ? t('servers.swap.creating') : t('servers.swap.create')}
			</Button>
			<Button size="sm" variant="outline" onclick={() => (showManual = !showManual)}>
				{t('servers.swap.manual')}
			</Button>
		</div>
		{#if failure}
			<p class="text-xs text-destructive">{failure}</p>
		{/if}
		{#if showManual}
			<p class="text-xs">{t('servers.swap.manual_hint')}</p>
			<div class="flex items-start gap-2 rounded-md border bg-muted/50 p-2 font-mono text-xs">
				<code class="min-w-0 flex-1 break-all whitespace-pre-wrap">{MANUAL_COMMANDS}</code>
				<CopyButton
					value={MANUAL_COMMANDS}
					label={null}
					revealOnHover={false}
					class="mt-0.5 shrink-0 text-muted-foreground hover:text-foreground" />
			</div>
		{/if}
	</Alert.Description>
</Alert.Root>
