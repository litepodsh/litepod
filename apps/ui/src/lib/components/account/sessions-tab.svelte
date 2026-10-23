<script lang="ts">
	import SkeletonRows from '#lib/components/custom/skeleton-rows.svelte';
	import SettingsSection from '#lib/components/projects/settings-section.svelte';
	import { toneBadge, toneDot, type StatusTone } from '#lib/utils/status-tone.js';
	import { onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as AlertDialog from '#lib/components/ui/alert-dialog/index.js';
	import DeleteConfirmControl from '#lib/components/custom/delete-confirm-control.svelte';
	import LogOut from '@lucide/svelte/icons/log-out';
	import CopyButton from '#lib/components/custom/copy-button.svelte';
	import { toast, toastError } from '#lib/utils/toast.js';
	import { UseAsyncResource } from '#lib/hooks/use-async-resource.svelte.js';
	import {
		listSessions,
		revokeSession,
		revokeAllSessions,
		revokeOtherSessions,
		type SessionDto
	} from '#lib/clients/account-client.js';
	import SessionDevice from './session-device.svelte';
	import AgentIcon from '#lib/components/icons/agent-icon.svelte';
	import { t } from '#lib/i18n/index.svelte.js';

	const sessionsResource = new UseAsyncResource(
		() => listSessions(),
		t('account.sessions.load_failed'),
	);
	let sessions = $derived(sessionsResource.data ?? []);
	let revokingAll = $state(false);
	let revokingOthers = $state(false);

	onMount(() => sessionsResource.load());

	/** A session row is open until it expires or its grant is closed. */
	function isExpired(session: SessionDto): boolean {
		return new Date(session.expires_at + 'Z').getTime() < Date.now();
	}
	function isClosed(session: SessionDto): boolean {
		return session.kind === 'mcp' && (session.closed_at !== null || isExpired(session));
	}

	function badgeFor(session: SessionDto): { tone: StatusTone; label: string } {
		if (isClosed(session)) return { tone: 'neutral', label: t('account.sessions.closed') };
		if (isExpired(session)) return { tone: 'danger', label: t('account.common.expired') };
		if (session.is_current) return { tone: 'ok', label: t('account.sessions.current') };
		if (session.kind === 'mcp') return { tone: 'ok', label: t('account.sessions.mcp_open') };
		return { tone: 'neutral', label: t('account.sessions.active') };
	}

	async function handleRevoke(id: string) {
		const session = sessions.find((s) => s.id === id);
		const confirmKey =
			session?.kind === 'mcp'
				? 'account.sessions.revoke_mcp_confirm'
				: 'account.sessions.revoke_confirm';
		if (!confirm(t(confirmKey))) return;
		try {
			await revokeSession(id);
			if (session?.kind === 'mcp') {
				// The record stays as history, now marked closed.
				sessionsResource.data = sessions.map((s) =>
					s.id === id ? { ...s, closed_at: new Date().toISOString().slice(0, 19).replace('T', ' ') } : s,
				);
			} else {
				sessionsResource.data = sessions.filter((s) => s.id !== id);
			}
			toast.success(t('account.sessions.revoked'));
		} catch (err) {
			toastError(err, t('account.sessions.revoke_failed'));
		}
	}

	async function handleRevokeOthers() {
		if (!confirm(t('account.sessions.revoke_others_confirm'))) return;
		revokingOthers = true;
		try {
			await revokeOtherSessions();
			// Other devices vanish; agent grants stay listed, marked closed.
			sessionsResource.data = sessions
				.filter((s) => s.is_current || s.kind === 'mcp')
				.map((s) =>
					s.kind === 'mcp'
						? { ...s, closed_at: new Date().toISOString().slice(0, 19).replace('T', ' ') }
						: s,
				);
			toast.success(t('account.sessions.others_revoked'));
		} catch (err) {
			toastError(err, t('account.sessions.revoke_others_failed'));
		} finally {
			revokingOthers = false;
		}
	}

	async function handleRevokeAll() {
		revokingAll = true;
		try {
			await revokeAllSessions();
			toast.success(t('account.sessions.all_revoked'));
			goto('/login');
		} catch (err) {
			revokingAll = false;
			toastError(err, t('account.sessions.revoke_all_failed'));
		}
	}
</script>

<SettingsSection
	title={t('account.sessions.title')}
	description={t('account.sessions.description')}>
	{#snippet aside()}
		{#if sessions.length > 0}
			<div class="mt-3 flex flex-wrap items-center gap-2">
				{#if sessions.some((s) => !s.is_current && !isClosed(s))}
					<Button
						variant="outline"
						size="sm"
						onclick={handleRevokeOthers}
						disabled={revokingOthers}>
						{revokingOthers ? t('account.sessions.revoking') : t('account.sessions.revoke_others')}
					</Button>
				{/if}
				<AlertDialog.Root>
					<AlertDialog.Trigger>
						{#snippet child({ props })}
							<Button {...props} variant="destructive" size="sm" disabled={revokingAll}>
								<LogOut />
								{t('account.sessions.revoke_all')}
							</Button>
						{/snippet}
					</AlertDialog.Trigger>
					<AlertDialog.Content>
						<AlertDialog.Header>
							<AlertDialog.Title>{t('account.sessions.revoke_all_title')}</AlertDialog.Title>
							<AlertDialog.Description>
								{t('account.sessions.revoke_all_description')}
							</AlertDialog.Description>
						</AlertDialog.Header>
						<AlertDialog.Footer>
							<AlertDialog.Cancel>{t('account.common.cancel')}</AlertDialog.Cancel>
							<DeleteConfirmControl
								label={t('account.sessions.revoke_all')}
								pendingLabel={t('account.sessions.revoking')}
								disabled={revokingAll}
								onconfirm={handleRevokeAll} />
						</AlertDialog.Footer>
					</AlertDialog.Content>
				</AlertDialog.Root>
			</div>
		{/if}
	{/snippet}

	{#if sessionsResource.loading}
		<SkeletonRows rows={3} height="h-16" label={t('account.sessions.loading')} />
	{:else if sessions.length === 0}
		<p class="rounded-[10px] bg-muted/40 px-3 py-4 text-muted-foreground">
			{t('account.sessions.empty')}
		</p>
	{:else}
		<ul class="flex flex-col gap-1.5">
			{#each sessions as session (session.id)}
				{@const badge = badgeFor(session)}
				<li
					class="flex flex-col gap-2 rounded-[10px] border border-foreground/9 px-3 py-2.5 sm:flex-row sm:items-center sm:justify-between">
					<div class="flex min-w-0 flex-col gap-1">
						<div class="flex flex-wrap items-center gap-2">
							{#if session.kind === 'mcp'}
								<div class="flex items-center gap-2">
									<AgentIcon agent={session.agent} class="size-4" />
									<span class="text-sm">
										{t('account.sessions.mcp_connection', {
											client: session.client_name ?? t('account.sessions.mcp_unknown_client')
										})}
									</span>
								</div>
							{:else}
								<SessionDevice userAgent={session.user_agent} />
							{/if}
							<span
								class="inline-flex items-center gap-1.5 rounded-full border px-2 py-0.5 text-xs font-medium {toneBadge(
									badge.tone,
								)}">
								<span class="size-1.5 rounded-full {toneDot(badge.tone)}"></span>
								{badge.label}
							</span>
						</div>
						<dl class="flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-muted-foreground">
							<div class="flex items-center gap-1">
								<dt class="sr-only">ID</dt>
								<dd>
									<CopyButton
										value={session.id}
										label="{session.id.slice(0, 8)}…"
										title={session.id}
										errorMessage="Failed to copy session id"
										class="max-w-[140px] px-1 py-0.5 font-mono hover:bg-muted"
										iconClass="size-3" />
								</dd>
							</div>
							{#if session.created_at}
								<div class="flex items-center gap-1">
									<dt>{t('account.sessions.created')}</dt>
									<dd class="tabular-nums">
										{new Date(session.created_at + 'Z').toLocaleString()}
									</dd>
								</div>
							{/if}
							{#if session.closed_at}
								<div class="flex items-center gap-1">
									<dt>{t('account.sessions.closed_at')}</dt>
									<dd class="tabular-nums">
										{new Date(session.closed_at + 'Z').toLocaleString()}
									</dd>
								</div>
							{:else}
								<div class="flex items-center gap-1">
									<dt>{t('account.sessions.expires')}</dt>
									<dd class="tabular-nums">
										{new Date(session.expires_at + 'Z').toLocaleString()}
									</dd>
								</div>
							{/if}
						</dl>
					</div>
					{#if !session.is_current && !isClosed(session)}
						<Button
							variant="ghost"
							size="sm"
							class="shrink-0 self-start text-destructive hover:text-destructive sm:self-auto"
							onclick={() => handleRevoke(session.id)}>
							{t('account.common.revoke')}
						</Button>
					{/if}
				</li>
			{/each}
		</ul>
	{/if}
</SettingsSection>
