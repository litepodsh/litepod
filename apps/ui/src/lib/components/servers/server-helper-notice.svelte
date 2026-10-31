<script lang="ts">
	import LoaderCircleIcon from '@lucide/svelte/icons/loader-circle';
	import TerminalIcon from '@lucide/svelte/icons/terminal';
	import * as Alert from '#lib/components/ui/alert/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { updateServerHelper, type Server } from '#lib/clients/server-client.js';
	import { toast } from '#lib/utils/toast.js';
	import { t } from '#lib/i18n/index.svelte.js';

	let {
		organizationId,
		server,
		onUpdated,
	}: {
		organizationId: string;
		server: Server;
		/** Re-read the server so the notice clears. */
		onUpdated: () => void;
	} = $props();

	const helper = $derived(server.facts.helper);

	let updating = $state(false);
	let failure = $state('');

	async function update() {
		updating = true;
		failure = '';
		try {
			const result = await updateServerHelper(organizationId, server.id);
			if (result.success) {
				toast.success(t('servers.helper.updated_toast', { name: server.name }));
				onUpdated();
			} else {
				failure = (result.stderr || result.stdout).trim() || t('servers.helper.failed');
			}
		} catch (caught) {
			failure = caught instanceof Error ? caught.message : t('servers.helper.failed');
		} finally {
			updating = false;
		}
	}
</script>

{#if helper}
	<Alert.Root>
		<TerminalIcon />
		<Alert.Title>{t('servers.helper.title')}</Alert.Title>
		<Alert.Description class="space-y-3">
			<p>
				{t('servers.helper.explanation', {
					installed: helper.installed,
					latest: helper.latest,
				})}
			</p>
			<ul class="space-y-2 text-xs">
				{#each helper.changes as release (release.version)}
					<li>
						<span class="font-medium">v{release.version} · {release.summary}</span>
						<ul class="list-disc pl-4 text-muted-foreground">
							{#each release.changes as change (change)}
								<li>{change}</li>
							{/each}
						</ul>
					</li>
				{/each}
			</ul>
			<Button size="sm" disabled={updating} onclick={() => void update()}>
				{#if updating}<LoaderCircleIcon class="size-4 animate-spin" />{/if}
				{updating ? t('servers.helper.updating') : t('servers.helper.update')}
			</Button>
			{#if failure}
				<p class="text-xs text-destructive">{failure}</p>
			{/if}
		</Alert.Description>
	</Alert.Root>
{/if}
