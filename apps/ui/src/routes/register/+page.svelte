<script lang="ts">
	import { goto } from '$app/navigation';
	import { onMount } from 'svelte';
	import { z } from 'zod';
	import * as Card from '#lib/components/ui/card/index.js';
	import logo from '#lib/assets/favicon.svg';
	import {
		FieldDescription,
		FieldGroup,
		Field,
		FieldLabel,
	} from '#lib/components/ui/field/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Checkbox } from '#lib/components/ui/checkbox/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { cn } from '#lib/utils.js';
	import {
		getRegistrationStatus,
		register,
		setup,
		type RegistrationStatusResponse,
	} from '#lib/clients/auth-client.js';
	import { acceptInvitation, inspectInvitation } from '#lib/clients/invitation-client.js';
	import { TURNSTILE_SITE_KEY } from '#lib/clients/base-client.js';
	import { registrationSubmitMode } from '#lib/utils/cloud-registration.js';
	import { loadTurnstileScript, readTurnstileToken, resetTurnstile } from '#lib/utils/turnstile.js';
	import { toast } from '#lib/utils/toast.js';
	import type { HTMLAttributes } from 'svelte/elements';
	import { t } from '#lib/i18n/index.svelte.js';

	let { class: className, ...restProps }: HTMLAttributes<HTMLDivElement> = $props();
	const id = Math.random().toString(36).slice(2);
	let name = $state('');
	let email = $state('');
	let password = $state('');
	let confirmPassword = $state('');
	let error = $state('');
	let loading = $state(false);
	let inviteToken = $state('');
	let checking = $state(true);
	let registrationStatus = $state<RegistrationStatusResponse | null>(null);
	let invitationUnavailable = $state(false);
	let turnstileEl = $state<HTMLDivElement | undefined>();
	/** Cloud only. The API refuses a cloud sign-up without it, so this box is a
	 *  courtesy that explains the refusal before it happens, not the check. */
	let acceptTerms = $state(false);
	const termsRequired = $derived(registrationStatus?.is_cloud ?? false);

	const schema = z
		.object({
			name: z.string().min(1, t('auth.register.username_required')),
			email: z.string().email(t('auth.register.invalid_email')),
			password: z.string().min(8, t('auth.register.password_short')),
			confirmPassword: z.string().min(1, t('auth.register.password_confirmation')),
		})
		.refine((data) => data.password === data.confirmPassword, {
			message: t('auth.register.password_mismatch'),
			path: ['confirmPassword'],
		});

	onMount(async () => {
		const token = new URLSearchParams(window.location.search).get('token');
		try {
			if (token) {
				inviteToken = token;
				getRegistrationStatus()
					.then((status) => (registrationStatus = status))
					.catch(() => {});
				const invitation = await inspectInvitation(token);
				name = invitation.username;
				email = invitation.email;
			} else {
				registrationStatus = await getRegistrationStatus();
				if (registrationStatus.is_cloud) loadTurnstileScript();
			}
		} catch (err) {
			invitationUnavailable = !!token;
			error = err instanceof Error ? err.message : t('auth.register.invalid_invitation');
		} finally {
			checking = false;
		}
	});

	async function handleSubmit(event: SubmitEvent) {
		event.preventDefault();
		if (loading) return;
		error = '';
		const result = schema.safeParse({ name, email, password, confirmPassword });
		if (!result.success) {
			error = result.error.issues[0].message;
			return;
		}
		if (termsRequired && !acceptTerms) {
			error = t('auth.register.accept_terms_required');
			return;
		}
		loading = true;
		try {
			if (inviteToken) {
				await acceptInvitation(inviteToken, password, confirmPassword, acceptTerms);
				toast.success(t('auth.register.account_activated'));
				goto('/home');
			} else if (
				registrationStatus &&
				registrationSubmitMode(registrationStatus, false) === 'register'
			) {
				const turnstileToken = readTurnstileToken(turnstileEl);
				if (registrationStatus.is_cloud && !turnstileToken) {
					error = t('auth.register.verification_challenge');
					loading = false;
					return;
				}
				const result = await register(
					name,
					email,
					password,
					confirmPassword,
					turnstileToken,
					acceptTerms,
				);
				if ('accepted_deliveries' in result) {
					sessionStorage.setItem('pending-email-verification', JSON.stringify(result));
					goto('/verify-email');
				} else {
					toast.success(t('auth.register.account_organization_created'));
					goto('/home');
				}
			} else {
				await setup(name, email, password, confirmPassword);
				toast.success(t('auth.register.administrator_created'));
				goto('/home');
			}
		} catch (err) {
			error = err instanceof Error ? err.message : t('auth.register.something_went_wrong');
			if (
				/invitation (has been revoked|has expired|has already been accepted|not found)/i.test(error)
			) {
				invitationUnavailable = true;
			}
		} finally {
			if (registrationStatus?.is_cloud) resetTurnstile(turnstileEl);
			loading = false;
		}
	}
</script>

<div
	class={cn(
		'bg-muted flex min-h-svh flex-col items-center justify-center gap-6 p-6 md:p-10',
		className,
	)}
	{...restProps}>
	<div class="w-full max-w-sm md:max-w-3xl">
		<Card.Root class="overflow-hidden p-0">
			<Card.Content class="grid p-0 md:grid-cols-2">
				{#if checking}
					<div class="flex items-center justify-center p-6 md:p-8">
						<div class="text-muted-foreground">{t('auth.register.checking')}</div>
					</div>
				{:else if invitationUnavailable}
					<div class="flex flex-col items-center justify-center gap-4 p-6 text-center md:p-8">
						<h1 class="text-2xl font-bold">{t('auth.register.invitation_unavailable')}</h1>
						<p class="text-destructive text-sm">{error}</p>
						<Button href="/login">{t('auth.register.go_to_login')}</Button>
					</div>
				{:else if error && !inviteToken && !registrationStatus?.public_registration_open}
					<div class="flex flex-col items-center justify-center gap-4 p-6 text-center md:p-8">
						<h1 class="text-2xl font-bold">{t('auth.register.registration_by_invitation')}</h1>
						<p class="text-destructive text-sm">{error}</p>
						<Button href="/login">{t('auth.register.go_to_login')}</Button>
					</div>
				{:else if !inviteToken && !registrationStatus?.public_registration_open}
					<div class="flex flex-col items-center justify-center gap-4 p-6 text-center md:p-8">
						<h1 class="text-2xl font-bold">{t('auth.register.registration_by_invitation')}</h1>
						<p class="text-muted-foreground">
							{t('auth.register.invitation_description')}
						</p>
						<Button href="/login">{t('auth.register.go_to_login')}</Button>
					</div>
				{:else}
					<form onsubmit={handleSubmit} class="p-6 md:p-8">
						<FieldGroup>
							<div class="flex flex-col items-center gap-2 text-center">
								<h1 class="text-2xl font-bold">
									{inviteToken
										? t('auth.register.accept_invitation')
										: registrationStatus?.is_cloud
											? t('auth.register.create_your_account_organization')
											: t('auth.register.create_first_administrator')}
								</h1>
								<p class="text-muted-foreground text-balance">
									{inviteToken
										? t('auth.register.set_password')
										: registrationStatus?.is_cloud
											? t('auth.register.your_account_organization')
											: t('auth.register.setup_first_administrator')}
								</p>
							</div>
							{#if error}<p class="text-destructive text-center text-sm font-medium">
									{error}
								</p>{/if}
							<Field
								><FieldLabel for="name-{id}">{t('auth.register.username')}</FieldLabel><Input
									id="name-{id}"
									autocomplete="nickname"
									disabled={loading || !!inviteToken}
									bind:value={name} /></Field>
							<Field
								><FieldLabel for="email-{id}">{t('auth.register.email')}</FieldLabel><Input
									id="email-{id}"
									type="email"
									autocomplete="username"
									disabled={loading || !!inviteToken}
									bind:value={email} /></Field>
							<Field
								><FieldLabel for="password-{id}">{t('auth.register.password')}</FieldLabel><Input
									id="password-{id}"
									type="password"
									autocomplete="new-password"
									disabled={loading}
									bind:value={password} /></Field>
							<Field
								><FieldLabel for="confirm-password-{id}"
									>{t('auth.register.confirm_password')}</FieldLabel
								><Input
									id="confirm-password-{id}"
									type="password"
									autocomplete="new-password"
									disabled={loading}
									bind:value={confirmPassword} /></Field>
							{#if termsRequired}
								<Field orientation="horizontal">
									<Checkbox id="accept-terms-{id}" bind:checked={acceptTerms} disabled={loading} />
									<FieldLabel for="accept-terms-{id}" class="text-sm font-normal leading-snug">
										<span>
											{t('auth.register.accept_terms_prefix')}
											<a href="/terms" target="_blank" class="underline"
												>{t('auth.login.terms_of_service')}</a
											>,
											<a href="/acceptable-use" target="_blank" class="underline"
												>{t('auth.register.acceptable_use_policy')}</a>
											{t('auth.login.terms_connector')}
											<a href="/privacy" target="_blank" class="underline"
												>{t('auth.login.privacy_policy')}</a
											>.
										</span>
									</FieldLabel>
								</Field>
							{/if}
							{#if !inviteToken && registrationStatus?.is_cloud}
								<div
									class="cf-turnstile"
									data-sitekey={TURNSTILE_SITE_KEY}
									data-action="turnstile-spin-v2"
									bind:this={turnstileEl}>
								</div>
							{/if}
							<Field
								><Button type="submit" class="w-full" disabled={loading}
									>{loading
										? t('auth.register.save')
										: inviteToken
											? t('auth.register.activate_account')
											: registrationStatus?.is_cloud
												? t('auth.register.create_account_organization')
												: t('auth.register.create_administrator')}</Button
								></Field>
						</FieldGroup>
					</form>
				{/if}
				<div class="relative hidden overflow-hidden md:block">
					<div class="absolute inset-0 bg-gradient-to-br from-[#EEF0FF] via-[#E6E7FC] to-[#F1E9FF]">
					</div>
					<div class="absolute inset-0 flex items-center justify-center">
						<img
							src={logo}
							alt="litepod"
							class="h-48 w-48 drop-shadow-[0_8px_24px_rgba(91,95,239,0.25)]" />
					</div>
				</div>
			</Card.Content>
		</Card.Root>
		<FieldDescription class="px-6 text-center">
			{#if registrationStatus?.is_cloud}
				{t('auth.register.continue_terms')} <a href="/terms">{t('auth.login.terms_of_service')}</a>,
				<a href="/acceptable-use">{t('legal.common.acceptable_use_policy')}</a>,
				<a href="/refund">{t('auth.login.refund_policy')}</a>
				{t('auth.login.terms_connector')}
				<a href="/privacy">{t('auth.login.privacy_policy')}</a>.
			{:else}
				{t('legal.common.self_hosted_notice')}
			{/if}
		</FieldDescription>
	</div>
</div>
