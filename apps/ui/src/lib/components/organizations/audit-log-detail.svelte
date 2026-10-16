<script lang="ts">
	import * as Dialog from '#lib/components/ui/dialog/index.js';
	import * as Avatar from '#lib/components/ui/avatar/index.js';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import CopyIcon from '@lucide/svelte/icons/copy';
	import CheckIcon from '@lucide/svelte/icons/check';
	import { avatarUrl, initials } from '#lib/utils/avatar.js';
	import { formatAuditAction, type AuditLogDto } from '#lib/clients/audit-log-client.js';
	import AuditMetadataNode from './audit-metadata-node.svelte';
	import { t } from '#lib/i18n/index.svelte.js';

	let {
		open = $bindable(false),
		entry,
	}: {
		open: boolean;
		entry: AuditLogDto | null;
	} = $props();

	let copiedField = $state<string | null>(null);

	function formatDate(value: string): string {
		return new Date(value.endsWith('Z') ? value : `${value}Z`).toLocaleString();
	}

	function formatRelative(value: string): string {
		const date = new Date(value.endsWith('Z') ? value : `${value}Z`);
		const diffMs = Date.now() - date.getTime();
		const mins = Math.floor(diffMs / 60000);
		if (mins < 1) return t('organizations.audit_logs.just_now');
		if (mins < 60) return t('organizations.audit_logs.minutes_ago', { count: mins });
		const hours = Math.floor(mins / 60);
		if (hours < 24) return t('organizations.audit_logs.hours_ago', { count: hours });
		const days = Math.floor(hours / 24);
		if (days < 30) return t('organizations.audit_logs.days_ago', { count: days });
		return date.toLocaleDateString();
	}

	async function copy(text: string, field: string) {
		try {
			await navigator.clipboard.writeText(text);
			copiedField = field;
			setTimeout(() => {
				if (copiedField === field) copiedField = null;
			}, 1500);
		} catch {
			/* clipboard not available */
		}
	}

	function parseMetadata(raw: string | null): unknown {
		if (!raw) return null;
		try {
			return JSON.parse(raw);
		} catch {
			return raw;
		}
	}

	const metadata = $derived(entry ? parseMetadata(entry.metadata) : null);

	type LoginMeta = { method: string; mfa?: boolean; mfa_method?: string; sso_protocol?: string };
	const login = $derived(
		entry?.action === 'auth.login' &&
			metadata &&
			typeof metadata === 'object' &&
			'method' in metadata
			? (metadata as LoginMeta)
			: null,
	);

	function loginMethod(m: LoginMeta): string {
		if (m.method === 'sso')
			return t('organizations.audit_logs.login_method_sso', {
				protocol: (m.sso_protocol ?? '').toUpperCase(),
			});
		if (m.method === 'password') return t('organizations.audit_logs.login_method_password');
		return m.method;
	}

	function login2fa(m: LoginMeta): string {
		if (m.method === 'sso') return t('organizations.audit_logs.login_2fa_idp');
		if (!m.mfa) return t('organizations.audit_logs.login_2fa_none');
		if (m.mfa_method === 'recovery_code')
			return t('organizations.audit_logs.login_2fa_recovery_code');
		return t('organizations.audit_logs.login_2fa_totp');
	}
</script>

<Dialog.Root bind:open>
	<Dialog.Content
		class="flex w-full flex-col gap-5 overflow-y-auto p-0 sm:max-w-2xl"
		showCloseButton={false}>
		{#if entry}
			<Dialog.Header
				class="flex-row items-start justify-between gap-3 space-y-0 border-b px-6 pt-6 pb-4">
				<div class="min-w-0 flex-1">
					<Dialog.Title class="text-lg">
						{formatAuditAction(entry.action)}
					</Dialog.Title>
					<Dialog.Description class="font-mono text-xs">
						{entry.action}
					</Dialog.Description>
				</div>
				{#if entry.outcome}
					<Badge variant={entry.outcome === 'success' ? 'secondary' : 'destructive'}>
						{entry.outcome}
					</Badge>
				{/if}
			</Dialog.Header>

			<div class="flex flex-col gap-5 px-6 pb-6">
				<div class="grid grid-cols-1 gap-5 sm:grid-cols-2">
					<section class="flex flex-col gap-1.5">
						<h3 class="text-muted-foreground text-xs font-medium tracking-wide uppercase">
							{t('organizations.audit_logs.when')}
						</h3>
						<p class="text-sm">{formatDate(entry.created_at)}</p>
						<p class="text-muted-foreground text-xs">{formatRelative(entry.created_at)}</p>
					</section>

					<section class="flex flex-col gap-2">
						<h3 class="text-muted-foreground text-xs font-medium tracking-wide uppercase">
							{t('organizations.audit_logs.who')}
						</h3>
						<div class="flex items-center gap-2">
							<Avatar.Root class="size-8 rounded-lg">
								{#if entry.actor_email}
									<Avatar.Image
										src={avatarUrl(entry.actor_name || entry.actor_email)}
										alt={t('organizations.common.avatar_alt', {
											name: entry.actor_name || entry.actor_email,
										})} />
								{/if}
								<Avatar.Fallback class="rounded-lg text-xs">
									{entry.actor_email ? initials(entry.actor_name || entry.actor_email) : '?'}
								</Avatar.Fallback>
							</Avatar.Root>
							<div class="min-w-0 flex-1">
								<p class="truncate text-sm font-medium">
									{entry.actor_name ??
										entry.actor_email ??
										t('organizations.audit_logs.deleted_user')}
								</p>
								{#if entry.actor_email}
									<p class="text-muted-foreground truncate text-xs">{entry.actor_email}</p>
								{/if}
							</div>
						</div>
						{#if entry.actor_token_name}
							<div
								class="bg-muted/30 flex items-center justify-between rounded-md px-2 py-1.5 text-xs">
								<span class="text-muted-foreground">
									{t('organizations.audit_logs.via_api_token')}
								</span>
								<code class="font-mono">{entry.actor_token_name}</code>
							</div>
						{/if}
					</section>

					<section class="flex flex-col gap-1.5">
						<h3 class="text-muted-foreground text-xs font-medium tracking-wide uppercase">
							{t('organizations.audit_logs.what')}
						</h3>
						{#if entry.target_name}
							<p class="text-sm font-medium">{entry.target_name}</p>
						{:else}
							<p class="text-muted-foreground text-sm italic">
								{t('organizations.audit_logs.no_target_name')}
							</p>
						{/if}
						{#if entry.target_type}
							<p class="text-muted-foreground text-xs">
								<span
									>{t('organizations.audit_logs.target_type_label', {
										type: entry.target_type,
									})}</span>
								{#if entry.target_id}
									<span class="ml-2 inline-flex items-center gap-1">
										<code class="font-mono">{entry.target_id}</code>
										<button
											type="button"
											class="text-muted-foreground hover:text-foreground"
											onclick={() => copy(entry.target_id ?? '', 'target_id')}
											aria-label={t('organizations.audit_logs.copy_target_id')}>
											<CopyIcon class="size-3" />
										</button>
									</span>
								{/if}
							</p>
						{/if}
					</section>

					<section class="flex flex-col gap-1.5">
						<h3 class="text-muted-foreground text-xs font-medium tracking-wide uppercase">
							{t('organizations.audit_logs.where')}
						</h3>
						{#if entry.source_ip || entry.user_agent || entry.request_id}
							{#if entry.source_ip}
								<div class="flex items-center justify-between text-xs">
									<span class="text-muted-foreground">{t('organizations.audit_logs.ip')}</span>
									<code class="font-mono">{entry.source_ip}</code>
								</div>
							{/if}
							{#if entry.user_agent}
								<div class="flex items-start justify-between gap-2 text-xs">
									<span class="text-muted-foreground shrink-0">
										{t('organizations.audit_logs.agent')}
									</span>
									<code class="font-mono break-all text-right">{entry.user_agent}</code>
								</div>
							{/if}
							{#if entry.request_id}
								<div
									class="bg-muted/30 flex items-center justify-between gap-2 rounded-md px-2 py-1.5">
									<div class="min-w-0 flex-1">
										<p class="text-muted-foreground text-xs">
											{t('organizations.audit_logs.request_id')}
										</p>
										<code class="font-mono text-xs break-all">{entry.request_id}</code>
									</div>
									<Button
										size="icon"
										variant="ghost"
										class="size-7 shrink-0"
										onclick={() => copy(entry.request_id ?? '', 'request_id')}
										aria-label={t('organizations.audit_logs.copy_request_id')}>
										{#if copiedField === 'request_id'}
											<CheckIcon class="size-3.5 text-green-600" />
										{:else}
											<CopyIcon class="size-3.5" />
										{/if}
									</Button>
								</div>
							{/if}
						{:else}
							<p class="text-muted-foreground text-sm italic">
								{t('organizations.audit_logs.no_provenance')}
							</p>
						{/if}
					</section>
				</div>

				{#if login}
					<section class="flex flex-col gap-1.5">
						<h3 class="text-muted-foreground text-xs font-medium tracking-wide uppercase">
							{t('organizations.audit_logs.login_title')}
						</h3>
						<div class="bg-muted/30 flex flex-col gap-1.5 rounded-md p-3 text-sm">
							<div class="flex items-center justify-between gap-2">
								<span class="text-muted-foreground"
									>{t('organizations.audit_logs.login_method')}</span>
								<span class="font-medium">{loginMethod(login)}</span>
							</div>
							<div class="flex items-center justify-between gap-2">
								<span class="text-muted-foreground">{t('organizations.audit_logs.login_2fa')}</span>
								{#if login.mfa}
									<Badge variant="secondary">{login2fa(login)}</Badge>
								{:else}
									<span class="font-medium">{login2fa(login)}</span>
								{/if}
							</div>
						</div>
					</section>
				{:else if metadata !== null}
					<section class="flex flex-col gap-1.5">
						<h3 class="text-muted-foreground text-xs font-medium tracking-wide uppercase">
							{t('organizations.audit_logs.details')}
						</h3>
						{#if typeof metadata === 'string'}
							<pre
								class="bg-muted/30 max-h-48 overflow-auto rounded-md p-2 font-mono text-[11px] leading-snug">{metadata}</pre>
						{:else}
							<div class="bg-muted/30 rounded-md p-3">
								<AuditMetadataNode value={metadata} />
							</div>
						{/if}
					</section>
				{/if}

				{#if entry.error_message}
					<section class="flex flex-col gap-1.5">
						<h3 class="text-muted-foreground text-xs font-medium tracking-wide uppercase">
							{t('organizations.audit_logs.error')}
						</h3>
						<pre
							class="bg-destructive/10 text-destructive max-h-48 overflow-auto rounded-md p-2 font-mono text-[11px] leading-snug">{entry.error_message}</pre>
					</section>
				{/if}
			</div>

			<div class="bg-muted/20 border-t flex items-center justify-end gap-2 px-6 py-3">
				<Dialog.Close>
					{#snippet child({ props })}
						<Button variant="ghost" size="sm" {...props}>
							{t('organizations.common.close')}
						</Button>
					{/snippet}
				</Dialog.Close>
			</div>
		{/if}
	</Dialog.Content>
</Dialog.Root>
