<script lang="ts">
	import { goto } from '$app/navigation';
	import { onMount, tick } from 'svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import { t } from '#lib/i18n/index.svelte.js';
	import {
		confirmEmailVerification,
		resendEmailVerification,
		type PendingEmailVerification,
	} from '#lib/clients/auth-client.js';

	let pending = $state<PendingEmailVerification | null>(null);
	let message = $state(t('auth.verify_email.waiting_for_code'));
	let loading = $state(false);
	let now = $state(Date.now() / 1000);
	let verified = $state(false);
	let emailFromUrl = $state('');

	const slots: string[] = $state(['', '', '', '', '', '']);
	let slotRefs: HTMLInputElement[] = [];

	onMount(() => {
		emailFromUrl = new URLSearchParams(window.location.search).get('email') ?? '';
		try {
			pending = JSON.parse(sessionStorage.getItem('pending-email-verification') ?? 'null');
		} catch {
			pending = null;
		}
		if (!emailFromUrl && pending?.email) emailFromUrl = pending.email;
		// `reactivating` is always false on the wire now — a true would have told the
		// browser that this address has a deactivated account. The message stays generic.
		const timer = window.setInterval(() => (now = Date.now() / 1000), 1000);
		void tick().then(() => slotRefs[0]?.focus());
		return () => window.clearInterval(timer);
	});

	function handleInput(index: number, event: Event) {
		const el = event.target as HTMLInputElement;
		let value = el.value.toUpperCase();
		value = value.replace(/[^A-Z0-9]/g, '');
		if (value.length > 1) {
			const chars = value.slice(0, 6).split('');
			for (let i = 0; i < 6; i++) {
				slots[i] = chars[i] ?? '';
			}
			slotRefs[5]?.focus();
		} else {
			slots[index] = value;
			if (value && index < 5) slotRefs[index + 1]?.focus();
		}
		el.value = value.slice(-1);
	}

	function handleKeydown(index: number, event: KeyboardEvent) {
		if (event.key === 'Backspace' && !slots[index] && index > 0) {
			slots[index - 1] = '';
			slotRefs[index - 1]?.focus();
		} else if (event.key === 'ArrowLeft' && index > 0) {
			slotRefs[index - 1]?.focus();
		} else if (event.key === 'ArrowRight' && index < 5) {
			slotRefs[index + 1]?.focus();
		}
	}

	async function handleSubmit(event: Event) {
		event.preventDefault();
		if (loading || !emailFromUrl) return;
		const code = slots.join('').trim();
		if (code.length !== 6) {
			message = t('auth.verify_email.code_incomplete');
			return;
		}
		loading = true;
		try {
			const result = await confirmEmailVerification(emailFromUrl, code);
			verified = true;
			message = result.reactivated
				? t('auth.verify_email.restored')
				: t('auth.verify_email.email_verified');
		} catch (err) {
			message = err instanceof Error ? err.message : t('auth.verify_email.invalid_code');
		} finally {
			loading = false;
		}
	}

	async function resend() {
		if (!pending || loading || pending.resend_available_at > now) return;
		loading = true;
		try {
			await resendEmailVerification(pending.email);
			pending = {
				...pending,
				accepted_deliveries: pending.accepted_deliveries + 1,
				resend_available_at: Math.floor(Date.now() / 1000) + 60,
			};
			sessionStorage.setItem('pending-email-verification', JSON.stringify(pending));
			message = t('auth.verify_email.new_code_sent');
			for (let i = 0; i < 6; i++) slots[i] = '';
			slotRefs[0]?.focus();
		} catch {
			message = t('auth.verify_email.resend_unavailable');
		} finally {
			loading = false;
		}
	}
</script>

<main class="bg-muted flex min-h-svh items-center justify-center p-6">
	<section class="bg-background w-full max-w-md rounded-lg border p-8 text-center shadow-sm">
		<h1 class="text-2xl font-bold">{t('auth.verify_email.verify_email')}</h1>

		{#if !verified}
			{#if emailFromUrl}
				<p class="text-muted-foreground mt-3 text-sm">{emailFromUrl}</p>
			{/if}

			<p class="text-muted-foreground mt-3">{message}</p>

			{#if emailFromUrl || pending?.email}
				<form onsubmit={handleSubmit} class="mt-6">
					<div class="flex justify-center gap-2">
						{#each [0, 1, 2, 3, 4, 5] as i}
							<input
								bind:this={slotRefs[i]}
								type="text"
								inputmode="text"
								maxlength="1"
								class="border-input bg-background ring-offset-background placeholder:text-muted-foreground focus-visible:ring-ring w-12 rounded-md border px-0 py-2 text-center text-2xl font-mono uppercase focus-visible:ring-2 focus-visible:ring-offset-2 focus-visible:outline-none disabled:cursor-not-allowed disabled:opacity-50"
								value={slots[i]}
								disabled={loading}
								oninput={(e) => handleInput(i, e)}
								onkeydown={(e) => handleKeydown(i, e)}
								onpaste={(e) => {
									e.preventDefault();
									const paste = e.clipboardData?.getData('text') ?? '';
									const chars = paste
										.toUpperCase()
										.replace(/[^A-Z0-9]/g, '')
										.split('')
										.slice(0, 6);
									for (let j = 0; j < 6; j++) slots[j] = chars[j] ?? '';
									slotRefs[Math.min(chars.length, 5)]?.focus();
								}} />
						{/each}
					</div>
					<Button type="submit" class="mt-6 w-full" disabled={loading}>
						{loading ? t('auth.verify_email.verifying') : t('auth.verify_email.verify_code')}
					</Button>
				</form>
			{/if}

			{#if pending}
				<Button
					class="mt-4 w-full"
					variant="outline"
					onclick={resend}
					disabled={loading ||
						pending.accepted_deliveries >= 4 ||
						pending.resend_available_at > now}>
					{pending.resend_available_at > now
						? t('auth.verify_email.resend_in', {
								seconds: Math.ceil(pending.resend_available_at - now),
							})
						: t('auth.verify_email.resend_code', {
								count: Math.max(0, 4 - pending.accepted_deliveries),
							})}
				</Button>
			{/if}
		{:else}
			<p class="text-muted-foreground mt-3">{message}</p>
			<Button class="mt-6 w-full" onclick={() => goto('/login')}
				>{t('auth.verify_email.go_to_login')}</Button>
		{/if}
	</section>
</main>
