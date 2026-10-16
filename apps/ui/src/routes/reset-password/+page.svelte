<script lang="ts">
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import * as Card from '#lib/components/ui/card/index.js';
	import {
		FieldGroup,
		Field,
		FieldLabel,
		FieldDescription,
	} from '#lib/components/ui/field/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { confirmPasswordReset } from '#lib/clients/auth-client.js';
	import { ApiError } from '#lib/clients/base-client.js';
	import { t } from '#lib/i18n/index.svelte.js';

	const id = $props.id();

	const token = $derived(page.url.searchParams.get('token') ?? '');

	let password = $state('');
	let confirmPassword = $state('');
	let loading = $state(false);
	let error = $state('');
	let done = $state(false);

	async function handleSubmit(e: SubmitEvent) {
		e.preventDefault();
		if (password.length < 8) {
			error = t('auth.reset_password.password_short');
			return;
		}
		if (password !== confirmPassword) {
			error = t('auth.reset_password.password_mismatch');
			return;
		}
		error = '';
		loading = true;
		try {
			await confirmPasswordReset(token, password, confirmPassword);
			done = true;
		} catch (err) {
			// 401 covers expired, already-used and never-issued alike — the API
			// deliberately does not distinguish them.
			error =
				err instanceof ApiError && err.status === 401
					? t('auth.reset_password.link_expired')
					: err instanceof Error
						? err.message
						: t('auth.reset_password.reset_failed');
		} finally {
			loading = false;
		}
	}
</script>

<div class="bg-muted flex min-h-svh flex-col items-center justify-center p-6 md:p-10">
	<div class="flex w-full max-w-sm flex-col gap-6">
		<Card.Root class="p-0">
			<Card.Content class="p-6 md:p-8">
				{#if !token}
					<FieldGroup>
						<div class="flex flex-col items-center gap-2 text-center">
							<h1 class="text-2xl font-bold">{t('auth.reset_password.link_incomplete')}</h1>
							<p class="text-muted-foreground text-balance">
								{t('auth.reset_password.link_needed')}
							</p>
						</div>
						<FieldDescription class="text-center">
							<a href="/forgot-password">{t('auth.reset_password.request_new_link')}</a>
						</FieldDescription>
					</FieldGroup>
				{:else if done}
					<FieldGroup>
						<div class="flex flex-col items-center gap-2 text-center">
							<h1 class="text-2xl font-bold">{t('auth.reset_password.password_updated')}</h1>
							<p class="text-muted-foreground text-balance">
								{t('auth.reset_password.sessions_signed_out')}
							</p>
						</div>
						<Field>
							<Button class="w-full" onclick={() => goto('/login')}
								>{t('auth.reset_password.go_to_sign_in')}</Button>
						</Field>
					</FieldGroup>
				{:else}
					<form onsubmit={handleSubmit}>
						<FieldGroup>
							<div class="flex flex-col items-center gap-2 text-center">
								<h1 class="text-2xl font-bold">{t('auth.reset_password.set_new_password')}</h1>
							</div>

							{#if error}
								<p class="text-destructive text-center text-sm font-medium">{error}</p>
							{/if}

							<Field>
								<FieldLabel for="password-{id}">{t('auth.reset_password.new_password')}</FieldLabel>
								<Input
									id="password-{id}"
									type="password"
									autocomplete="new-password"
									disabled={loading}
									bind:value={password} />
							</Field>

							<Field>
								<FieldLabel for="confirm-{id}"
									>{t('auth.reset_password.confirm_password')}</FieldLabel>
								<Input
									id="confirm-{id}"
									type="password"
									autocomplete="new-password"
									disabled={loading}
									bind:value={confirmPassword} />
							</Field>

							<Field>
								<Button type="submit" class="w-full" disabled={loading}>
									{loading
										? t('auth.reset_password.save')
										: t('auth.reset_password.set_new_password')}
								</Button>
							</Field>
						</FieldGroup>
					</form>
				{/if}
			</Card.Content>
		</Card.Root>
	</div>
</div>
