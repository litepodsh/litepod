<script lang="ts">
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Alert from '#lib/components/ui/alert/index.js';
	import * as Dialog from '#lib/components/ui/dialog/index.js';
	import { Label } from '#lib/components/ui/label/index.js';
	import ArrowUpCircleIcon from '@lucide/svelte/icons/circle-arrow-up';
	import TriangleAlertIcon from '@lucide/svelte/icons/triangle-alert';
	import {
		catalogUpdateApply,
		catalogUpdateGet,
		catalogUpdateIgnore,
		type CatalogUpdate,
	} from '#lib/clients/project-client.js';
	import { toast } from '#lib/utils/toast.js';
	import { t } from '#lib/i18n/index.svelte.js';

	let {
		organizationId,
		projectId,
		environmentId,
		applicationId,
		canUpdate = true,
		onApplied,
	}: {
		organizationId: string;
		projectId: string;
		environmentId: string;
		applicationId: string;
		canUpdate?: boolean;
		/** Called after the update is stored so the page can reload compose and env. */
		onApplied?: () => void | Promise<void>;
	} = $props();

	let update = $state<CatalogUpdate | null>(null);
	let open = $state(false);
	let busy = $state(false);
	let replaceCompose = $state(true);
	let addEnv = $state(true);

	const composeChanged = $derived(update?.compose.status !== 'unchanged');
	const customized = $derived(update?.compose.status === 'customized');
	const visible = $derived(!!update?.update_available);

	$effect(() => {
		void applicationId;
		let active = true;
		// The catalog is a network call: a failure must not break the page.
		void catalogUpdateGet(organizationId, projectId, environmentId, applicationId)
			.then((result) => {
				if (!active) return;
				update = result;
				replaceCompose = result?.compose.status === 'clean';
				addEnv = true;
			})
			.catch(() => {
				if (active) update = null;
			});
		return () => {
			active = false;
		};
	});

	async function apply() {
		if (!update) return;
		busy = true;
		try {
			update = await catalogUpdateApply(organizationId, projectId, environmentId, applicationId, {
				version: update.latest_version,
				compose: replaceCompose && composeChanged ? 'replace' : 'keep',
				add_env: addEnv,
			});
			open = false;
			toast.success(t('projects.catalog_update.applied'));
			await onApplied?.();
		} catch (caught) {
			toast.error(caught instanceof Error ? caught.message : t('projects.catalog_update.failed'));
		} finally {
			busy = false;
		}
	}

	async function ignore() {
		if (!update) return;
		busy = true;
		try {
			await catalogUpdateIgnore(
				organizationId,
				projectId,
				environmentId,
				applicationId,
				update.latest_version,
			);
			update = { ...update, update_available: false, ignored: true };
			open = false;
		} catch (caught) {
			toast.error(caught instanceof Error ? caught.message : t('projects.catalog_update.failed'));
		} finally {
			busy = false;
		}
	}
</script>

{#if visible && update}
	<Alert.Root class="mt-2">
		<ArrowUpCircleIcon />
		<Alert.Title>
			{t('projects.catalog_update.title', { version: update.latest_version })}
		</Alert.Title>
		<Alert.Description>
			{update.current_version
				? t('projects.catalog_update.from', { version: update.current_version })
				: t('projects.catalog_update.from_unknown')}
			{#if update.env.added.length > 0}
				{t('projects.catalog_update.new_variables', { count: update.env.added.length })}
			{/if}
			<div class="mt-2 flex gap-2">
				<Button size="sm" onclick={() => (open = true)}>
					{t('projects.catalog_update.review')}
				</Button>
				{#if canUpdate}
					<Button size="sm" variant="ghost" disabled={busy} onclick={ignore}>
						{t('projects.catalog_update.skip')}
					</Button>
				{/if}
			</div>
		</Alert.Description>
	</Alert.Root>

	<Dialog.Root bind:open>
		<Dialog.Content class="max-h-[85vh] overflow-y-auto sm:max-w-3xl">
			<Dialog.Header>
				<Dialog.Title>
					{t('projects.catalog_update.dialog_title', { version: update.latest_version })}
				</Dialog.Title>
				<Dialog.Description>{t('projects.catalog_update.dialog_description')}</Dialog.Description>
			</Dialog.Header>

			<section class="grid gap-2">
				<h3 class="text-sm font-medium">{t('projects.catalog_update.environment')}</h3>
				{#if update.env.added.length === 0 && update.env.removed.length === 0 && update.env.changed_defaults.length === 0}
					<p class="text-muted-foreground text-sm">{t('projects.catalog_update.env_none')}</p>
				{/if}
				{#if update.env.added.length > 0}
					<div class="rounded-lg border p-3">
						<Label class="flex items-start gap-2">
							<input type="checkbox" class="mt-1" bind:checked={addEnv} disabled={!canUpdate} />
							<span>
								{t('projects.catalog_update.env_added')}
								<span class="text-muted-foreground block text-xs font-normal">
									{t('projects.catalog_update.env_added_hint')}
								</span>
							</span>
						</Label>
						<ul class="mt-2 grid gap-1 font-mono text-xs">
							{#each update.env.added as variable (variable.name)}
								<li class="text-emerald-600 dark:text-emerald-400">
									+ {variable.name}={variable.value}
								</li>
							{/each}
						</ul>
					</div>
				{/if}
				{#if update.env.removed.length > 0}
					<div class="rounded-lg border p-3 text-xs">
						<p class="mb-1 text-sm">{t('projects.catalog_update.env_removed')}</p>
						<ul class="font-mono">
							{#each update.env.removed as name (name)}
								<li class="text-red-600 dark:text-red-400">- {name}</li>
							{/each}
						</ul>
					</div>
				{/if}
				{#if update.env.changed_defaults.length > 0}
					<div class="rounded-lg border p-3 text-xs">
						<p class="mb-1 text-sm">{t('projects.catalog_update.env_defaults')}</p>
						<ul class="font-mono">
							{#each update.env.changed_defaults as change (change.name)}
								<li>{change.name}: {change.from} → {change.to}</li>
							{/each}
						</ul>
					</div>
				{/if}
			</section>

			<section class="grid gap-2">
				<h3 class="text-sm font-medium">{t('projects.catalog_update.compose')}</h3>
				{#if !composeChanged}
					<p class="text-muted-foreground text-sm">{t('projects.catalog_update.compose_none')}</p>
				{:else}
					{#if update.compose.images.length > 0}
						<ul class="grid gap-1 font-mono text-xs">
							{#each update.compose.images as image (image.service)}
								<li>
									{image.service}: {image.from ?? '—'} → {image.to ?? '—'}
								</li>
							{/each}
						</ul>
					{/if}
					{#if customized}
						<Alert.Root variant="destructive">
							<TriangleAlertIcon />
							<Alert.Description>
								{update.has_baseline
									? t('projects.catalog_update.customized')
									: t('projects.catalog_update.no_baseline')}
							</Alert.Description>
						</Alert.Root>
					{/if}
					<pre
						class="bg-muted max-h-80 overflow-auto rounded-lg p-3 font-mono text-xs leading-relaxed">{#each update.compose.diff as line, index (index)}<span
								class={line.tag === 'insert'
									? 'block bg-emerald-500/15 text-emerald-700 dark:text-emerald-300'
									: line.tag === 'delete'
										? 'block bg-red-500/15 text-red-700 dark:text-red-300'
										: 'text-muted-foreground block'}
								>{line.tag === 'insert'
									? '+ '
									: line.tag === 'delete'
										? '- '
										: '  '}{line.text}</span
							>{/each}</pre>
					<Label class="flex items-center gap-2">
						<input type="checkbox" bind:checked={replaceCompose} disabled={!canUpdate} />
						{customized
							? t('projects.catalog_update.replace_customized')
							: t('projects.catalog_update.replace')}
					</Label>
				{/if}
			</section>

			{#if update.requires_https || update.url_persisted}
				<p class="text-muted-foreground text-xs">
					{t('projects.catalog_update.url_note')}
				</p>
			{/if}
			<p class="text-muted-foreground text-xs">{t('projects.catalog_update.redeploy_note')}</p>

			<Dialog.Footer>
				<Button variant="ghost" disabled={busy} onclick={() => (open = false)}>
					{t('projects.projects.cancel')}
				</Button>
				{#if canUpdate}
					<Button variant="outline" disabled={busy} onclick={ignore}>
						{t('projects.catalog_update.skip')}
					</Button>
					<Button disabled={busy} onclick={apply}>
						{t('projects.catalog_update.apply')}
					</Button>
				{/if}
			</Dialog.Footer>
		</Dialog.Content>
	</Dialog.Root>
{/if}
