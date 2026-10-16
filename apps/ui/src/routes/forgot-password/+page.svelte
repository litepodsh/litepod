<script lang="ts">
	import * as Card from '#lib/components/ui/card/index.js';
	import {
		FieldGroup,
		Field,
		FieldLabel,
		FieldDescription,
	} from '#lib/components/ui/field/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { requestPasswordReset } from '#lib/clients/auth-client.js';
	import { z } from 'zod';
	import { t } from '#lib/i18n/index.svelte.js';

	const schema = z.object({ email: z.email(t('auth.forgot_password.email_invalid')) });

	const id = $props.id();

	let email = $state('');
	let loading = $state(false);
	let error = $state('');
	// The API answers the same for a known and an unknown address, so this screen
	// must too — anything else would turn the page into an account-existence oracle.
	let submitted = $state(false);

	async function handleSubmit(e: SubmitEvent) {
		e.preventDefault();
		const parsed = schema.safeParse({ email });
		if (!parsed.success) {
			error = parsed.error.issues[0]?.message ?? t('auth.forgot_password.email_invalid');
			return;
		}
		error = '';
		loading = true;
		try {
			await requestPasswordReset(email);
			submitted = true;
		} catch (err) {
			error = err instanceof Error ? err.message : t('auth.forgot_password.send_failed');
		} finally {
			loading = false;
		}
	}
</script>

<div class="bg-muted flex min-h-svh flex-col items-center justify-center p-6 md:p-10">
	<div class="flex w-full max-w-sm flex-col gap-6">
		<Card.Root class="p-0">
			<Card.Content class="p-6 md:p-8">
				{#if submitted}
					<FieldGroup>
						<div class="flex flex-col items-center gap-2 text-center">
							<h1 class="text-2xl font-bold">{t('auth.forgot_password.check_email')}</h1>
							<p class="text-muted-foreground text-balance">
								{t('auth.forgot_password.email_sent', { email })}
							</p>
						</div>
						<FieldDescription class="text-center">
							<a href="/login">{t('auth.forgot_password.back_to_sign_in')}</a>
						</FieldDescription>
					</FieldGroup>
				{:else}
					<form onsubmit={handleSubmit}>
						<FieldGroup>
							<div class="flex flex-col items-center gap-2 text-center">
								<h1 class="text-2xl font-bold">{t('auth.forgot_password.forgot_password')}</h1>
								<p class="text-muted-foreground text-balance">
									{t('auth.forgot_password.enter_email')}
								</p>
							</div>

							{#if error}
								<p class="text-destructive text-center text-sm font-medium">{error}</p>
							{/if}

							<Field>
								<FieldLabel for="email-{id}">{t('auth.forgot_password.email')}</FieldLabel>
								<Input
									id="email-{id}"
									type="email"
									placeholder="m@example.com"
									disabled={loading}
									bind:value={email} />
							</Field>

							<Field>
								<Button type="submit" class="w-full" disabled={loading}>
									{loading
										? t('auth.forgot_password.sending')
										: t('auth.forgot_password.send_reset_link')}
								</Button>
							</Field>

							<FieldDescription class="text-center">
								{t('auth.forgot_password.remembered')}
								<a href="/login">{t('auth.forgot_password.sign_in')}</a>
							</FieldDescription>
						</FieldGroup>
					</form>
				{/if}
			</Card.Content>
		</Card.Root>
	</div>
</div>
