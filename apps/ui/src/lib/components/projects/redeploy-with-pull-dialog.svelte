<script lang="ts">
	import * as AlertDialog from '#lib/components/ui/alert-dialog/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import HoldToDelete from '#lib/components/custom/hold-to-delete.svelte';
	import SlideToDelete from '#lib/components/custom/slide-to-delete.svelte';
	import { IsMobile } from '#lib/hooks/is-mobile.svelte.js';
	import RocketIcon from '@lucide/svelte/icons/rocket';
	import { toast } from '#lib/utils/toast.js';
	import { t } from '#lib/i18n/index.svelte.js';

	let {
		open = $bindable(false),
		applicationName,
		ondeploy,
	}: {
		open?: boolean;
		applicationName: string;
		ondeploy: (confirmation: string) => Promise<void>;
	} = $props();

	let deploying = $state(false);
	let confirmKey = $state(0);
	const isMobile = new IsMobile();

	function reset() {
		deploying = false;
	}

	$effect(() => {
		if (!open) reset();
	});

	async function redeploy() {
		if (deploying) return;
		deploying = true;
		try {
			await ondeploy(applicationName);
			open = false;
		} catch (error) {
			toast.error(error instanceof Error ? error.message : t('projects.projects.redeploy_failed'));
		} finally {
			deploying = false;
			confirmKey += 1;
		}
	}
</script>

<AlertDialog.Root bind:open>
	<AlertDialog.Content>
		<AlertDialog.Header>
			<AlertDialog.Title>{t('projects.projects.redeploy_latest')}</AlertDialog.Title>
			<AlertDialog.Description>
				{t('projects.projects.redeploy_latest_hint')}
			</AlertDialog.Description>
		</AlertDialog.Header>

		<div class="grid gap-6">
			<AlertDialog.Footer>
				<Button type="button" variant="outline" disabled={deploying} onclick={() => (open = false)}>
					{t('projects.projects.cancel')}
				</Button>
				{#key confirmKey}
					{#if isMobile.current}
						<SlideToDelete
							label={t('projects.projects.slide_redeploy')}
							doneLabel={t('projects.projects.redeploying')}
							variant="primary"
							icon={RocketIcon}
							disabled={deploying}
							onconfirm={() => void redeploy()} />
					{:else}
						<HoldToDelete
							label={t('projects.projects.hold_redeploy')}
							doneLabel={t('projects.projects.redeploying')}
							variant="primary"
							icon={RocketIcon}
							class="sm:h-8 sm:w-auto"
							disabled={deploying}
							onconfirm={() => void redeploy()} />
					{/if}
				{/key}
			</AlertDialog.Footer>
		</div>
	</AlertDialog.Content>
</AlertDialog.Root>
