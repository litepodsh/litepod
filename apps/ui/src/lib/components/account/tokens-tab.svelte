<script lang="ts">
	import ChevronDownIcon from '@lucide/svelte/icons/chevron-down';
	import * as Empty from '#lib/components/ui/empty/index.js';
	import KeyRoundIcon from '@lucide/svelte/icons/key-round';
	import SkeletonRows from '#lib/components/custom/skeleton-rows.svelte';
	import SettingsSection from '#lib/components/projects/settings-section.svelte';
	import { toneBadge } from '#lib/utils/status-tone.js';
	import { onMount } from 'svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Label } from '#lib/components/ui/label/index.js';
	import SoftCheckbox from '#lib/components/custom/soft-checkbox.svelte';
	import * as AlertDialog from '#lib/components/ui/alert-dialog/index.js';
	import DeleteConfirmControl from '#lib/components/custom/delete-confirm-control.svelte';
	import CopyButton from '#lib/components/custom/copy-button.svelte';
	import { toast, toastError } from '#lib/utils/toast.js';
	import { UseAsyncResource } from '#lib/hooks/use-async-resource.svelte.js';
	import {
		listTokens,
		createToken,
		revokeToken,
		revokeAllTokens,
		listGrantablePermissions,
	} from '#lib/clients/personal-access-token-client.js';
	import { groupCatalog } from '#lib/clients/access-client.js';
	import { t } from '#lib/i18n/index.svelte.js';

	const SCOPES = [{ value: 'read' }, { value: 'write' }, { value: 'deploy' }] as const;

	const DEFAULT_EXPIRY_DAYS = 30;
	const MAX_EXPIRY_DAYS = 365;

	function dateAt(daysFromNow: number) {
		const d = new Date();
		d.setDate(d.getDate() + daysFromNow);
		return d.toISOString().slice(0, 10);
	}
	const minDate = dateAt(1);
	const maxDate = dateAt(MAX_EXPIRY_DAYS);

	const tokensResource = new UseAsyncResource(() => listTokens(), t('account.tokens.load_failed'));
	let tokens = $derived(tokensResource.data ?? []);

	let grantable = $state<string[]>([]);
	const groups = $derived(groupCatalog(grantable.map((name) => ({ name, org_only: false }))));
	let granularOpen = $state(false);

	let name = $state('');
	let scopes = $state<Record<string, boolean>>({ read: true, write: false, deploy: false });
	let expiresAt = $state(dateAt(DEFAULT_EXPIRY_DAYS));
	let neverExpires = $state(false);
	let creating = $state(false);
	let revokingId = $state<string | null>(null);
	let revokingAll = $state(false);
	let createdToken = $state('');

	onMount(() => {
		tokensResource.load();
		listGrantablePermissions()
			.then((names) => (grantable = names))
			.catch(() => {});
	});

	function selectedScopes() {
		return Object.entries(scopes)
			.filter(([, checked]) => checked)
			.map(([scope]) => scope);
	}

	async function submit(event: SubmitEvent) {
		event.preventDefault();
		const chosen = selectedScopes();
		if (!name.trim() || chosen.length === 0 || creating) return;
		creating = true;
		try {
			const created = await createToken({
				name: name.trim(),
				scopes: chosen,
				expires_at: neverExpires ? null : expiresAt,
			});
			createdToken = created.token;
			tokensResource.data = [created.token_info, ...tokens];
			name = '';
			scopes = { read: true, write: false, deploy: false };
			granularOpen = false;
			expiresAt = dateAt(DEFAULT_EXPIRY_DAYS);
			neverExpires = false;
		} catch (err) {
			toastError(err, t('account.tokens.create_failed'));
		} finally {
			creating = false;
		}
	}

	async function handleRevoke(id: string) {
		if (!confirm(t('account.tokens.revoke_confirm'))) return;
		revokingId = id;
		try {
			await revokeToken(id);
			tokensResource.data = tokens.filter((t) => t.id !== id);
			toast.success(t('account.tokens.revoked'));
		} catch (err) {
			toastError(err, t('account.tokens.revoke_failed'));
		} finally {
			revokingId = null;
		}
	}

	async function handleRevokeAll() {
		revokingAll = true;
		try {
			await revokeAllTokens();
			tokensResource.data = [];
			toast.success(t('account.tokens.all_revoked'));
		} catch (err) {
			toastError(err, t('account.tokens.revoke_all_failed'));
		} finally {
			revokingAll = false;
		}
	}
</script>

<SettingsSection
	title={t('account.tokens.new_title')}
	description={t('account.tokens.new_description')}>
	<form class="flex flex-col gap-4" onsubmit={submit}>
		<div class="grid gap-1.5">
			<Label for="token-name">{t('account.tokens.name')}</Label>
			<Input
				id="token-name"
				placeholder={t('account.tokens.name_placeholder')}
				bind:value={name}
				maxlength={100} />
		</div>

		<div class="grid gap-1.5">
			<Label>{t('account.tokens.scopes')}</Label>
			<div class="grid gap-1.5 sm:grid-cols-3">
				{#each SCOPES as scope (scope.value)}
					<div
						class="flex items-start gap-2 rounded-[10px] border border-foreground/9 px-3 py-2.5 transition-colors has-[[data-state=checked]]:bg-muted/50 motion-reduce:transition-none">
						<SoftCheckbox
							id="token-scope-{scope.value}"
							bind:checked={scopes[scope.value]}
							class="mt-0.5" />
						<div class="grid gap-0.5">
							<Label for="token-scope-{scope.value}"
								>{t(`account.tokens.scope_labels.${scope.value}`)}</Label>
							<p class="text-xs text-muted-foreground">
								{t(`account.tokens.scope_hints.${scope.value}`)}
							</p>
						</div>
					</div>
				{/each}
			</div>
		</div>

		{#if grantable.length > 0}
			<details class="group rounded-[10px] border border-foreground/9" bind:open={granularOpen}>
				<summary
					class="flex cursor-pointer list-none items-center justify-between gap-2 px-3 py-2.5 font-medium">
					<span class="flex items-center gap-2">
						{t('account.tokens.granular_title')}
						{#if grantable.some((p) => scopes[p])}
							<span
								class="inline-flex items-center rounded-full border px-2 py-0.5 text-xs font-medium tabular-nums {toneBadge(
									'info',
								)}">
								{grantable.filter((p) => scopes[p]).length}
							</span>
						{/if}
					</span>
					<ChevronDownIcon
						class="size-4 text-muted-foreground transition-transform group-open:rotate-180 motion-reduce:transition-none" />
				</summary>
				<div class="lp-groove-t grid gap-3 p-3">
					<p class="text-xs text-muted-foreground">
						{t('account.tokens.granular_description')}
					</p>
					{#each groups as [resource, entries] (resource)}
						<fieldset class="grid gap-2 rounded-[10px] border border-foreground/9 p-3">
							<legend class="px-1 text-[12.5px] font-medium">{resource}</legend>
							<div class="grid gap-2 sm:grid-cols-2">
								{#each entries as entry (entry.name)}
									<label class="flex items-center gap-2">
										<!-- Granular scopes start absent from `scopes`; binding undefined to a prop with a fallback throws. -->
										<SoftCheckbox
											bind:checked={
												() => scopes[entry.name] ?? false, (value) => (scopes[entry.name] = value)
											} />
										<span class="min-w-0 font-mono text-xs break-all">{entry.name}</span>
									</label>
								{/each}
							</div>
						</fieldset>
					{/each}
				</div>
			</details>
		{/if}

		<div class="grid gap-1.5 sm:max-w-xs">
			<Label for="token-expires">{t('account.tokens.expires')}</Label>
			<Input
				id="token-expires"
				type="date"
				min={minDate}
				max={maxDate}
				class="font-mono tabular-nums"
				bind:value={expiresAt}
				disabled={neverExpires} />
			<div class="flex items-center gap-2 pt-1">
				<SoftCheckbox id="token-never-expires" bind:checked={neverExpires} />
				<Label for="token-never-expires" class="font-normal text-muted-foreground">
					{t('account.tokens.never_expires')}
				</Label>
			</div>
		</div>

		{#if createdToken}
			<div class="grid gap-2 rounded-[10px] border border-warning/40 bg-warning/10 px-3 py-2.5">
				<p class="font-medium">{t('account.tokens.copy_now')}</p>
				<CopyButton
					value={createdToken}
					label={createdToken}
					errorMessage={t('account.tokens.copy_failed')}
					class="w-fit max-w-full font-mono text-xs break-all" />
			</div>
		{/if}

		<div class="flex justify-end">
			<Button type="submit" disabled={!name.trim() || selectedScopes().length === 0 || creating}>
				{creating ? t('account.tokens.creating') : t('account.tokens.create')}
			</Button>
		</div>
	</form>
</SettingsSection>

<SettingsSection title={t('account.tokens.title')} description={t('account.tokens.description')}>
	{#snippet aside()}
		{#if tokens.length > 0}
			<div class="mt-3">
				<AlertDialog.Root>
					<AlertDialog.Trigger>
						{#snippet child({ props })}
							<Button {...props} variant="destructive" size="sm" disabled={revokingAll}>
								{t('account.tokens.revoke_all')}
							</Button>
						{/snippet}
					</AlertDialog.Trigger>
					<AlertDialog.Content>
						<AlertDialog.Header>
							<AlertDialog.Title>{t('account.tokens.revoke_all_title')}</AlertDialog.Title>
							<AlertDialog.Description>
								{t('account.tokens.revoke_all_description')}
							</AlertDialog.Description>
						</AlertDialog.Header>
						<AlertDialog.Footer>
							<AlertDialog.Cancel>{t('account.common.cancel')}</AlertDialog.Cancel>
							<DeleteConfirmControl
								label={t('account.tokens.revoke_all')}
								pendingLabel={t('account.tokens.revoking')}
								disabled={revokingAll}
								onconfirm={handleRevokeAll} />
						</AlertDialog.Footer>
					</AlertDialog.Content>
				</AlertDialog.Root>
			</div>
		{/if}
	{/snippet}

	{#if tokensResource.loading}
		<SkeletonRows rows={3} height="h-16" label={t('account.tokens.loading')} />
	{:else if tokens.length === 0}
		<Empty.Root class="rounded-[10px] border border-dashed border-foreground/15">
			<Empty.Header>
				<Empty.Media variant="icon"><KeyRoundIcon /></Empty.Media>
				<Empty.Title>{t('account.tokens.empty_title')}</Empty.Title>
				<Empty.Description>
					{t('account.tokens.empty_description')}
				</Empty.Description>
			</Empty.Header>
		</Empty.Root>
	{:else}
		<ul class="flex flex-col gap-1.5">
			{#each tokens as token (token.id)}
				{@const expired = token.expires_at
					? new Date(token.expires_at + 'Z').getTime() < Date.now()
					: false}
				<li
					class="flex flex-col gap-2 rounded-[10px] border border-foreground/9 px-3 py-2.5 sm:flex-row sm:items-center sm:justify-between">
					<div class="flex min-w-0 flex-col gap-1.5">
						<div class="flex min-w-0 flex-wrap items-center gap-2">
							<span class="truncate font-medium">{token.name}</span>
							<span class="font-mono text-xs text-muted-foreground">{token.token_prefix}…</span>
							{#if expired}
								<span
									class="inline-flex items-center rounded-full border px-2 py-0.5 text-xs font-medium {toneBadge(
										'danger',
									)}">{t('account.common.expired')}</span>
							{/if}
						</div>
						<div class="flex flex-wrap gap-1">
							{#each token.scopes as scope (scope)}
								<span
									class="inline-flex items-center rounded-[6px] bg-muted px-1.5 py-0.5 font-mono text-[11px] text-muted-foreground"
									>{scope}</span>
							{/each}
						</div>
						<dl class="flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-muted-foreground">
							<div class="flex items-center gap-1">
								<dt>{t('account.tokens.expires')}</dt>
								<dd class="tabular-nums">
									{token.expires_at
										? new Date(token.expires_at + 'Z').toLocaleDateString()
										: t('account.common.never')}
								</dd>
							</div>
							<div class="flex items-center gap-1">
								<dt>{t('account.tokens.last_used')}</dt>
								<dd class="tabular-nums">
									{token.last_used_at
										? new Date(token.last_used_at + 'Z').toLocaleString()
										: t('account.common.never')}
								</dd>
							</div>
						</dl>
					</div>
					<Button
						variant="ghost"
						size="sm"
						class="shrink-0 self-start text-destructive hover:text-destructive sm:self-auto"
						disabled={revokingId === token.id}
						onclick={() => handleRevoke(token.id)}>
						{t('account.common.revoke')}
					</Button>
				</li>
			{/each}
		</ul>
	{/if}
</SettingsSection>
