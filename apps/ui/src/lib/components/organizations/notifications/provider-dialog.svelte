<script lang="ts">
	import * as Dialog from '#lib/components/ui/dialog/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Label } from '#lib/components/ui/label/index.js';
	import SoftSwitch from '#lib/components/custom/soft-switch.svelte';
	import * as Tooltip from '#lib/components/ui/tooltip/index.js';
	import EyeIcon from '@lucide/svelte/icons/eye';
	import EyeOffIcon from '@lucide/svelte/icons/eye-off';
	import LoaderCircleIcon from '@lucide/svelte/icons/loader-circle';
	import CircleCheckIcon from '@lucide/svelte/icons/circle-check';
	import CircleAlertIcon from '@lucide/svelte/icons/circle-alert';
	import TriangleAlertIcon from '@lucide/svelte/icons/triangle-alert';
	import LockIcon from '@lucide/svelte/icons/lock';
	import SendIcon from '@lucide/svelte/icons/send';
	import ProviderIcon from './provider-icon.svelte';
	import {
		testUnsaved,
		type CatalogAction,
		type CatalogProvider,
		type NotificationAction,
		type NotificationProviderDto,
		type ProviderConfigInput,
		type ProviderKind,
	} from '#lib/clients/notification-client.js';
	import { validateConfig, hostHint } from './validation';
	import { t } from '#lib/i18n/index.svelte.js';

	let {
		open = $bindable(false),
		organizationId,
		provider = null,
		providers,
		actions,
		organizationIsRoot,
		existingNames = [],
		onSubmit,
	}: {
		open?: boolean;
		organizationId: string;
		/** `null` creates, otherwise edits in place. */
		provider?: NotificationProviderDto | null;
		providers: CatalogProvider[];
		actions: CatalogAction[];
		organizationIsRoot: boolean;
		existingNames?: string[];
		onSubmit: (input: {
			name: string;
			kind: ProviderKind;
			decoration: boolean;
			config?: ProviderConfigInput;
			actions: NotificationAction[];
		}) => Promise<void>;
	} = $props();

	const editing = $derived(provider !== null);

	let kind = $state<ProviderKind>('discord');
	let name = $state('');
	let webhookUrl = $state('');
	let botToken = $state('');
	let chatId = $state('');
	let decoration = $state(true);
	let selected = $state<NotificationAction[]>([]);
	let showSecret = $state(false);
	let saving = $state(false);
	let testing = $state(false);
	let testResult = $state<{ ok: boolean; message: string } | null>(null);
	let submitError = $state<string | null>(null);
	let touched = $state(false);
	let wasOpen = $state(false);

	const activeProvider = $derived(providers.find((entry) => entry.kind === kind) ?? providers[0]);
	const usesWebhookUrl = $derived(activeProvider?.uses_webhook_url ?? true);

	// On edit the credential fields start blank: the API never returns the
	// stored secret, and blank means "keep it".
	const config = $derived<ProviderConfigInput>(
		usesWebhookUrl ? { webhook_url: webhookUrl } : { bot_token: botToken, chat_id: chatId },
	);
	const credentialsUntouched = $derived(
		usesWebhookUrl ? webhookUrl.trim() === '' : botToken.trim() === '' && chatId.trim() === '',
	);
	const keepingStoredCredential = $derived(editing && credentialsUntouched);

	const nameError = $derived.by(() => {
		const value = name.trim();
		if (!value) return t('organizations.notifications.name_required');
		if (value.length > 60) return t('organizations.notifications.name_too_long');
		const clash = existingNames.some(
			(other) => other.toLowerCase() === value.toLowerCase() && other !== provider?.name,
		);
		return clash ? t('organizations.notifications.name_clash') : null;
	});

	// Mirrors the server-side allowlist so a mismatch is caught before the
	// request rather than after a round trip.
	const configError = $derived.by(() => {
		if (keepingStoredCredential) return null;
		return validateConfig(kind, config, providers);
	});

	const canSubmit = $derived(!nameError && !configError && !saving);
	const canTest = $derived(!configError && !credentialsUntouched && !testing);

	$effect(() => {
		if (open && !wasOpen) reset();
		wasOpen = open;
	});

	function reset() {
		kind = provider?.kind ?? 'discord';
		name = provider?.name ?? '';
		decoration = provider?.decoration ?? true;
		selected = [...(provider?.actions ?? [])];
		webhookUrl = '';
		botToken = '';
		chatId = '';
		showSecret = false;
		testResult = null;
		submitError = null;
		touched = false;
		saving = false;
		testing = false;
	}

	function pickKind(next: ProviderKind) {
		if (editing || kind === next) return;
		kind = next;
		// Credentials are per-provider; carrying a Slack URL into Discord would
		// only produce a confusing mismatch error.
		webhookUrl = '';
		botToken = '';
		chatId = '';
		testResult = null;
	}

	function isLocked(action: CatalogAction) {
		return action.global && !organizationIsRoot;
	}

	function toggleAction(action: CatalogAction) {
		if (isLocked(action)) return;
		selected = selected.includes(action.key)
			? selected.filter((key) => key !== action.key)
			: [...selected, action.key];
	}

	async function runTest() {
		touched = true;
		if (!canTest) return;
		testing = true;
		testResult = null;
		try {
			const result = await testUnsaved(organizationId, { kind, decoration, config });
			testResult = { ok: result.delivered, message: result.message };
		} catch (err) {
			testResult = { ok: false, message: err instanceof Error ? err.message : String(err) };
		} finally {
			testing = false;
		}
	}

	async function submit() {
		touched = true;
		if (!canSubmit) return;
		saving = true;
		submitError = null;
		try {
			await onSubmit({
				name: name.trim(),
				kind,
				decoration,
				config: keepingStoredCredential ? undefined : config,
				actions: selected,
			});
			open = false;
		} catch (err) {
			submitError = err instanceof Error ? err.message : String(err);
		} finally {
			saving = false;
		}
	}
</script>

<Dialog.Root bind:open>
	<Dialog.Content class="max-h-[90dvh] gap-0 overflow-y-auto sm:max-w-2xl">
		<Dialog.Header>
			<Dialog.Title>
				{editing
					? t('organizations.notifications.edit_title')
					: t('organizations.notifications.add')}
			</Dialog.Title>
			<Dialog.Description>
				{editing
					? t('organizations.notifications.edit_description')
					: t('organizations.notifications.description')}
			</Dialog.Description>
		</Dialog.Header>

		<div class="flex flex-col gap-6 py-4">
			<!-- Provider -->
			<section class="flex flex-col gap-3">
				<h3 class="text-sm font-medium">{t('organizations.notifications.select_provider')}</h3>
				<div class="grid grid-cols-2 gap-3 sm:grid-cols-4">
					{#each providers as entry (entry.kind)}
						<button
							type="button"
							disabled={editing && entry.kind !== kind}
							aria-pressed={kind === entry.kind}
							onclick={() => pickKind(entry.kind)}
							class="flex flex-col items-center justify-center gap-2 rounded-lg border p-4 transition
								{kind === entry.kind ? 'border-foreground ring-foreground/20 ring-2' : 'hover:bg-accent'}
								disabled:pointer-events-none disabled:opacity-40">
							<ProviderIcon kind={entry.kind} class="size-6" />
							<span class="text-center text-xs font-medium">{entry.label}</span>
						</button>
					{/each}
				</div>
				{#if editing}
					<p class="text-muted-foreground text-xs">
						{t('organizations.notifications.provider_locked')}
					</p>
				{/if}
			</section>

			<!-- Credentials -->
			<section class="flex flex-col gap-4">
				<h3 class="text-sm font-medium">{t('organizations.notifications.fill_fields')}</h3>

				<div class="flex flex-col gap-1.5">
					<Label for="notification-name">{t('organizations.common.name')}</Label>
					<Input
						id="notification-name"
						bind:value={name}
						placeholder={t('organizations.notifications.name_placeholder')}
						aria-invalid={touched && !!nameError}
						onblur={() => (touched = true)} />
					{#if touched && nameError}
						<p class="text-destructive text-xs">{nameError}</p>
					{/if}
				</div>

				{#if usesWebhookUrl}
					<div class="flex flex-col gap-1.5">
						<Label for="notification-webhook">{t('organizations.notifications.webhook_url')}</Label>
						<div class="relative">
							<Input
								id="notification-webhook"
								type={showSecret ? 'text' : 'password'}
								bind:value={webhookUrl}
								class="pr-10"
								placeholder={hostHint(kind)}
								aria-invalid={touched && !!configError}
								onblur={() => (touched = true)} />
							<button
								type="button"
								class="text-muted-foreground hover:text-foreground absolute inset-y-0 right-0 flex w-10 items-center justify-center"
								aria-label={showSecret
									? t('organizations.notifications.hide_webhook')
									: t('organizations.notifications.show_webhook')}
								onclick={() => (showSecret = !showSecret)}>
								{#if showSecret}<EyeOffIcon class="size-4" />{:else}<EyeIcon class="size-4" />{/if}
							</button>
						</div>
						{#if keepingStoredCredential}
							<p class="text-muted-foreground text-xs">
								{t('organizations.notifications.webhook_keep', { hint: provider?.config_hint })}
							</p>
						{:else if touched && configError}
							<p class="text-destructive text-xs">{configError}</p>
						{:else}
							<p class="text-muted-foreground text-xs">
								{t('organizations.notifications.webhook_hosts', {
									hosts: activeProvider?.allowed_hosts,
								})}
							</p>
						{/if}
					</div>
				{:else}
					<div class="flex flex-col gap-1.5">
						<Label for="notification-token">{t('organizations.notifications.bot_token')}</Label>
						<div class="relative">
							<Input
								id="notification-token"
								type={showSecret ? 'text' : 'password'}
								bind:value={botToken}
								class="pr-10"
								placeholder="123456789:AAHdqTcvCH1vGWJxfSeofSAs0K5PALDsaw"
								aria-invalid={touched && !!configError}
								onblur={() => (touched = true)} />
							<button
								type="button"
								class="text-muted-foreground hover:text-foreground absolute inset-y-0 right-0 flex w-10 items-center justify-center"
								aria-label={showSecret
									? t('organizations.notifications.hide_token')
									: t('organizations.notifications.show_token')}
								onclick={() => (showSecret = !showSecret)}>
								{#if showSecret}<EyeOffIcon class="size-4" />{:else}<EyeIcon class="size-4" />{/if}
							</button>
						</div>
						<p class="text-muted-foreground text-xs">
							{t('organizations.notifications.bot_hint_before')}<span class="font-mono"
								>@BotFather</span
							>{t('organizations.notifications.bot_hint_after')}
						</p>
					</div>
					<div class="flex flex-col gap-1.5">
						<Label for="notification-chat">{t('organizations.notifications.chat_id')}</Label>
						<Input
							id="notification-chat"
							bind:value={chatId}
							placeholder={t('organizations.notifications.chat_id_placeholder')}
							aria-invalid={touched && !!configError}
							onblur={() => (touched = true)} />
						{#if keepingStoredCredential}
							<p class="text-muted-foreground text-xs">
								{t('organizations.notifications.token_keep', { hint: provider?.config_hint })}
							</p>
						{:else if touched && configError}
							<p class="text-destructive text-xs">{configError}</p>
						{:else}
							<p class="text-muted-foreground text-xs">
								{t('organizations.notifications.chat_hint')}
							</p>
						{/if}
					</div>
				{/if}

				<div class="flex items-center justify-between rounded-lg border p-3">
					<div class="flex flex-col">
						<span class="text-sm font-medium">{t('organizations.notifications.decoration')}</span>
						<span class="text-muted-foreground text-xs">
							{t('organizations.notifications.decoration_hint')}
						</span>
					</div>
					<SoftSwitch
						bind:checked={decoration}
						aria-label={t('organizations.notifications.decoration_aria')} />
				</div>
			</section>

			<!-- Actions -->
			<section class="flex flex-col gap-3">
				<h3 class="text-sm font-medium">{t('organizations.notifications.select_actions')}</h3>
				<div class="grid gap-3 sm:grid-cols-2">
					{#each actions as action (action.key)}
						{@const locked = isLocked(action)}
						<Tooltip.Provider>
							<Tooltip.Root>
								<Tooltip.Trigger class="text-left">
									<div
										class="flex h-full items-start justify-between gap-3 rounded-lg border p-3 {locked
											? 'opacity-55'
											: ''}">
										<div class="flex flex-col gap-0.5">
											<span class="flex items-center gap-1.5 text-sm font-medium">
												{action.label}
												{#if locked}<LockIcon class="text-muted-foreground size-3" />{/if}
											</span>
											<span class="text-muted-foreground text-xs">{action.description}</span>
										</div>
										<SoftSwitch
											checked={selected.includes(action.key)}
											disabled={locked}
											onCheckedChange={() => toggleAction(action)}
											aria-label={action.label} />
									</div>
								</Tooltip.Trigger>
								{#if locked}
									<Tooltip.Content>
										{t('organizations.notifications.platform_root_only')}
									</Tooltip.Content>
								{/if}
							</Tooltip.Root>
						</Tooltip.Provider>
					{/each}
				</div>
				{#if selected.length === 0}
					<p
						class="flex items-start gap-2 rounded-lg border border-warning/40 bg-warning/10 p-2.5 text-xs text-warning">
						<TriangleAlertIcon class="mt-0.5 size-3.5 shrink-0" />
						{t('organizations.notifications.no_actions_warning')}
					</p>
				{/if}
			</section>

			{#if testResult}
				<p
					class="flex items-start gap-2 rounded-lg border p-2.5 text-xs {testResult.ok
						? 'border-success/40 bg-success/10 text-success'
						: 'border-destructive/40 bg-destructive/10 text-destructive'}">
					{#if testResult.ok}
						<CircleCheckIcon class="mt-0.5 size-3.5 shrink-0" />
					{:else}
						<CircleAlertIcon class="mt-0.5 size-3.5 shrink-0" />
					{/if}
					<span class="break-words">{testResult.message}</span>
				</p>
			{/if}

			{#if submitError}
				<p
					class="border-destructive/40 bg-destructive/10 text-destructive flex items-start gap-2 rounded-lg border p-2.5 text-xs">
					<CircleAlertIcon class="mt-0.5 size-3.5 shrink-0" />
					<span class="break-words">{submitError}</span>
				</p>
			{/if}
		</div>

		<Dialog.Footer class="flex-row items-center justify-between gap-2 sm:justify-between">
			<Button variant="outline" disabled={!canTest} onclick={runTest}>
				{#if testing}
					<LoaderCircleIcon class="size-4 animate-spin" />
				{:else}
					<SendIcon class="size-4" />
				{/if}
				{t('organizations.notifications.test')}
			</Button>
			<div class="flex items-center gap-2">
				<Button variant="ghost" onclick={() => (open = false)}>
					{t('organizations.common.cancel')}
				</Button>
				<Button disabled={!canSubmit} onclick={submit}>
					{#if saving}<LoaderCircleIcon class="size-4 animate-spin" />{/if}
					{editing ? t('organizations.common.save') : t('organizations.common.create')}
				</Button>
			</div>
		</Dialog.Footer>
	</Dialog.Content>
</Dialog.Root>
