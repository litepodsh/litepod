<script lang="ts">
	import { goto } from '$app/navigation';
	import { onMount } from 'svelte';
	import * as Card from '#lib/components/ui/card/index.js';
	import logo from '#lib/assets/favicon.svg';
	import {
		FieldGroup,
		Field,
		FieldLabel,
		FieldDescription,
		FieldSeparator,
	} from '#lib/components/ui/field/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import EyeIcon from '@lucide/svelte/icons/eye';
	import EyeOffIcon from '@lucide/svelte/icons/eye-off';
	import { cn } from '#lib/utils.js';
	import {
		login,
		completeTotpLogin,
		isTotpChallenge,
		checkSetup,
		getRegistrationStatus,
	} from '#lib/clients/auth-client.js';
	import { ApiError, TURNSTILE_SITE_KEY } from '#lib/clients/base-client.js';
	import { startSso } from '#lib/ee/sso/sso-client.js';
	import { page } from '$app/state';
	import { loadTurnstileScript, readTurnstileToken, resetTurnstile } from '#lib/utils/turnstile.js';
	import type { HTMLAttributes } from 'svelte/elements';
	import { z } from 'zod';
	import { t } from '#lib/i18n/index.svelte.js';

	const loginSchema = z.object({
		email: z.email(t('auth.login.email_invalid')),
		password: z.string().min(8, t('auth.login.password_short')),
	});

	type LoginField = keyof z.infer<typeof loginSchema>;
	type FieldErrors = Partial<Record<LoginField, string>>;

	let { class: className, ...restProps }: HTMLAttributes<HTMLDivElement> = $props();
	const id = $props.id();

	let email = $state('');
	let password = $state('');
	let showPassword = $state(false);
	let passwordInput: HTMLInputElement | null = $state(null);
	let error = $state('');
	let loading = $state(false);
	let setupRequired = $state(false);
	let fieldErrors: FieldErrors = $state({});
	let hasSubmitted = $state(false);
	let isCloud = $state(false);
	// Single sign-on is self-hosted only; hidden until we know the edition.
	let ssoAvailable = $state(false);
	let ssoLoading = $state(false);
	let captchaRequired = $state(false);
	let turnstileEl = $state<HTMLDivElement | undefined>();

	// Set once the password is accepted but the account has 2FA. While it holds a
	// value the form shows the code step instead; there is no session yet.
	let totpChallenge = $state<string | null>(null);
	let totpCode = $state('');
	let totpInput = $state<HTMLInputElement | null>(null);

	onMount(async () => {
		// The SSO callback lands here with the reason when sign-in failed.
		const ssoError = page.url.searchParams.get('sso_error');
		if (ssoError) error = ssoError;
		try {
			const setup = await checkSetup();
			setupRequired = setup.setup_required;
			if (setup.setup_required) {
				goto('/register');
				return;
			}
		} catch {
			// If check fails, let the user stay on the login page
		}
		try {
			isCloud = (await getRegistrationStatus()).is_cloud;
			ssoAvailable = !isCloud;
		} catch {
			// Bot-check is an enhancement, not a login blocker — stay usable if this fails.
		}
	});

	/** Where a successful login lands: the `next` path the 401 redirect sent, or home. */
	function afterLogin() {
		const next = page.url.searchParams.get('next');
		if (next && next.startsWith('/') && !next.startsWith('//')) {
			goto(next);
		} else {
			goto('/home');
		}
	}

	async function handleSubmit(e: SubmitEvent) {
		e.preventDefault();
		if (loading) return;
		error = '';
		hasSubmitted = true;

		const result = loginSchema.safeParse({ email, password });
		if (!result.success) {
			const errors: FieldErrors = {};
			for (const issue of result.error.issues) {
				const field = issue.path[0];
				if (field === 'email' || field === 'password') {
					errors[field] = issue.message;
				}
			}
			fieldErrors = errors;
			return;
		}
		fieldErrors = {};

		loading = true;
		try {
			const turnstileToken = captchaRequired ? readTurnstileToken(turnstileEl) : undefined;
			if (captchaRequired && !turnstileToken) {
				error = t('auth.login.verification_challenge');
				return;
			}
			const result = await login(email, password, turnstileToken);
			if (isTotpChallenge(result)) {
				totpChallenge = result.challenge_token;
				error = '';
				return;
			}
			afterLogin();
		} catch (err) {
			error = err instanceof Error ? err.message : t('auth.login.invalid_credentials');
			if (err instanceof ApiError && err.captchaRequired) {
				if (!captchaRequired) loadTurnstileScript();
				captchaRequired = true;
			}
			if (captchaRequired) resetTurnstile(turnstileEl);
		} finally {
			loading = false;
		}
	}

	async function handleTotpSubmit(e: SubmitEvent) {
		e.preventDefault();
		if (!totpChallenge) return;
		error = '';
		loading = true;
		try {
			await completeTotpLogin(totpChallenge, totpCode.trim());
			afterLogin();
		} catch (err) {
			error = err instanceof Error ? err.message : t('auth.login.invalid_code');
			totpCode = '';
			// A spent or expired challenge cannot be retried — send the user back
			// to the password step rather than leaving them on a dead form.
			if (err instanceof ApiError && err.status === 401) {
				totpChallenge = null;
				password = '';
				error = t('auth.login.too_long');
			}
			totpInput?.focus();
		} finally {
			loading = false;
		}
	}

	/** Sends the browser to the identity provider that serves this address. */
	async function handleSso() {
		error = '';
		const parsed = loginSchema.shape.email.safeParse(email);
		if (!parsed.success) {
			hasSubmitted = true;
			fieldErrors = { email: parsed.error.issues[0]?.message };
			return;
		}
		ssoLoading = true;
		try {
			window.location.assign(await startSso(email.trim()));
		} catch (err) {
			error =
				err instanceof ApiError && err.status === 401
					? t('auth.login.sso_unavailable')
					: err instanceof Error
						? err.message
						: t('auth.login.sso_unavailable');
			ssoLoading = false;
		}
	}

	function handleEmailKeydown(e: KeyboardEvent) {
		if (e.key === 'Tab' && !e.shiftKey) {
			e.preventDefault();
			passwordInput?.focus();
		}
	}

	function revalidateField(field: LoginField) {
		if (!hasSubmitted) return;

		const result = loginSchema.safeParse({ email, password });
		const issue = result.success
			? undefined
			: result.error.issues.find((candidate) => candidate.path[0] === field);

		fieldErrors = { ...fieldErrors, [field]: issue?.message };
	}
</script>

<div class="bg-muted flex min-h-svh flex-col items-center justify-center p-6 md:p-10">
	<div class={cn('flex w-full max-w-sm flex-col gap-6 md:max-w-3xl', className)} {...restProps}>
		<Card.Root class="overflow-hidden p-0">
			<Card.Content class="grid p-0 md:grid-cols-2">
				{#if totpChallenge}
					<form onsubmit={handleTotpSubmit} class="p-6 md:p-8">
						<FieldGroup>
							<div class="flex flex-col items-center gap-2 text-center">
								<h1 class="text-2xl font-bold">{t('auth.login.two_factor_authentication')}</h1>
								<p class="text-muted-foreground text-balance">
									{t('auth.login.two_factor_description')}
								</p>
							</div>

							{#if error}
								<p class="text-destructive text-sm text-center font-medium">{error}</p>
							{/if}

							<Field>
								<FieldLabel for="totp-{id}">{t('auth.login.code')}</FieldLabel>
								<!-- svelte-ignore a11y_autofocus -->
								<Input
									bind:ref={totpInput}
									id="totp-{id}"
									name="totp"
									inputmode="text"
									autocomplete="one-time-code"
									maxlength={8}
									autocapitalize="characters"
									spellcheck={false}
									placeholder="123456"
									disabled={loading}
									autofocus
									bind:value={totpCode} />
							</Field>

							<Field>
								<Button type="submit" class="w-full" disabled={loading || !totpCode.trim()}>
									{loading ? t('auth.login.verifying') : t('auth.login.verify')}
								</Button>
							</Field>

							<FieldDescription class="text-center">
								<button
									type="button"
									class="underline underline-offset-2"
									onclick={() => {
										totpChallenge = null;
										totpCode = '';
										error = '';
									}}>
									{t('auth.login.back_to_sign_in')}
								</button>
							</FieldDescription>

							{#if isCloud}
								<FieldDescription class="text-center">
									{t('auth.login.support_contact')}
									<a href="mailto:info@litepod.sh">info@litepod.sh</a>
								</FieldDescription>
							{/if}
						</FieldGroup>
					</form>
				{:else}
					<form onsubmit={handleSubmit} class="p-6 md:p-8">
						<FieldGroup>
							<div class="flex flex-col items-center gap-2 text-center">
								<h1 class="text-2xl font-bold">{t('auth.login.welcome_back')}</h1>
								<p class="text-muted-foreground text-balance">{t('auth.login.login_account')}</p>
							</div>

							{#if error}
								<p class="text-destructive text-sm text-center font-medium">{error}</p>
							{/if}

							<Field>
								<FieldLabel for="email-{id}">{t('auth.login.email')}</FieldLabel>
								<Input
									id="email-{id}"
									type="email"
									autocomplete="username"
									placeholder="m@example.com"
									disabled={loading}
									aria-invalid={fieldErrors.email ? 'true' : undefined}
									bind:value={email}
									onkeydown={handleEmailKeydown}
									oninput={() => revalidateField('email')} />
								{#if fieldErrors.email}
									<FieldDescription class="text-destructive">
										{fieldErrors.email}
									</FieldDescription>
								{/if}
							</Field>

							<Field>
								<div class="flex items-center">
									<FieldLabel for="password-{id}">{t('auth.login.password')}</FieldLabel>
									<a
										href="/forgot-password"
										class="ms-auto text-sm underline-offset-2 hover:underline">
										{t('auth.login.forgot_password')}
									</a>
								</div>
								<div class="relative">
									<Input
										bind:ref={passwordInput}
										id="password-{id}"
										type={showPassword ? 'text' : 'password'}
										autocomplete="current-password"
										disabled={loading}
										aria-invalid={fieldErrors.password ? 'true' : undefined}
										class="pr-10"
										bind:value={password}
										oninput={() => revalidateField('password')} />
									<Button
										type="button"
										variant="ghost"
										size="icon"
										class="absolute right-1 top-1/2 size-7 -translate-y-1/2"
										disabled={loading}
										onclick={() => (showPassword = !showPassword)}
										aria-label={showPassword
											? t('auth.login.hide_password')
											: t('auth.login.show_password')}>
										{#if showPassword}
											<EyeOffIcon class="size-4" />
										{:else}
											<EyeIcon class="size-4" />
										{/if}
									</Button>
								</div>
								{#if fieldErrors.password}
									<FieldDescription class="text-destructive">
										{fieldErrors.password}
									</FieldDescription>
								{/if}
							</Field>

							{#if isCloud && captchaRequired}
								<div
									class="cf-turnstile"
									data-sitekey={TURNSTILE_SITE_KEY}
									data-action="turnstile-spin-v2"
									bind:this={turnstileEl}>
								</div>
							{/if}

							<Field>
								<Button type="submit" class="w-full" disabled={loading}>
									{loading ? t('auth.login.logging_in') : t('auth.login.login')}
								</Button>
							</Field>
							{#if ssoAvailable}
								<FieldSeparator class="*:data-[slot=field-separator-content]:bg-card">
									{t('auth.login.or')}
								</FieldSeparator>
								<Field>
									<Button
										type="button"
										variant="outline"
										class="w-full"
										disabled={loading || ssoLoading}
										onclick={handleSso}>
										{ssoLoading
											? t('auth.login.sso_redirecting')
											: t('auth.login.continue_with_sso')}
									</Button>
								</Field>
							{/if}
							<!--
							<FieldSeparator class="*:data-[slot=field-separator-content]:bg-card">
								Or continue with
							</FieldSeparator>

							<Field class="grid grid-cols-3 gap-4">
								<Button variant="outline" type="button">
									<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" class="size-5">
										<path
											d="M12.152 6.896c-.948 0-2.415-1.078-3.96-1.04-2.04.027-3.91 1.183-4.961 3.014-2.117 3.675-.546 9.103 1.519 12.09 1.013 1.454 2.208 3.09 3.792 3.039 1.52-.065 2.09-.987 3.935-.987 1.831 0 2.35.987 3.96.948 1.637-.026 2.676-1.48 3.676-2.948 1.156-1.688 1.636-3.325 1.662-3.415-.039-.013-3.182-1.221-3.22-4.857-.026-3.04 2.48-4.494 2.597-4.559-1.429-2.09-3.623-2.324-4.39-2.376-2-.156-3.675 1.09-4.61 1.09zM15.53 3.83c.843-1.012 1.4-2.427 1.245-3.83-1.207.052-2.662.805-3.532 1.818-.78.896-1.454 2.338-1.273 3.714 1.338.104 2.715-.688 3.559-1.701"
											fill="currentColor"
										/>
									</svg>
									<span class="sr-only">Login with Apple</span>
								</Button>
								<Button variant="outline" type="button">
									<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" class="size-5">
										<path
											d="M12.48 10.92v3.28h7.84c-.24 1.84-.853 3.187-1.787 4.133-1.147 1.147-2.933 2.4-6.053 2.4-4.827 0-8.6-3.893-8.6-8.72s3.773-8.72 8.6-8.72c2.6 0 4.507 1.027 5.907 2.347l2.307-2.307C18.747 1.44 16.133 0 12.48 0 5.867 0 .307 5.387.307 12s5.56 12 12.173 12c3.573 0 6.267-1.173 8.373-3.36 2.16-2.16 2.84-5.213 2.84-7.667 0-.76-.053-1.467-.173-2.053H12.48z"
											fill="currentColor"
										/>
									</svg>
									<span class="sr-only">Login with Google</span>
								</Button>
								<Button variant="outline" type="button">
									<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" class="size-5">
										<path
											d="M6.915 4.03c-1.968 0-3.683 1.28-4.871 3.113C.704 9.208 0 11.883 0 14.449c0 .706.07 1.369.21 1.973a6.624 6.624 0 0 0 .265.86 5.297 5.297 0 0 0 .371.761c.696 1.159 1.818 1.927 3.593 1.927 1.497 0 2.633-.671 3.965-2.444.76-1.012 1.144-1.626 2.663-4.32l.756-1.339.186-.325c.061.1.121.196.183.3l2.152 3.595c.724 1.21 1.665 2.556 2.47 3.314 1.046.987 1.992 1.22 3.06 1.22 1.075 0 1.876-.355 2.455-.843a3.743 3.743 0 0 0 .81-.973c.542-.939.861-2.127.861-3.745 0-2.72-.681-5.357-2.084-7.45-1.282-1.912-2.957-2.93-4.716-2.93-1.047 0-2.088.467-3.053 1.308-.652.57-1.257 1.29-1.82 2.05-.69-.875-1.335-1.547-1.958-2.056-1.182-.966-2.315-1.303-3.454-1.303zm10.16 2.053c1.147 0 2.188.758 2.992 1.999 1.132 1.748 1.647 4.195 1.647 6.4 0 1.548-.368 2.9-1.839 2.9-.58 0-1.027-.23-1.664-1.004-.496-.601-1.343-1.878-2.832-4.358l-.617-1.028a44.908 44.908 0 0 0-1.255-1.98c.07-.109.141-.224.211-.327 1.12-1.667 2.118-2.602 3.358-2.602zm-10.201.553c1.265 0 2.058.791 2.675 1.446.307.327.737.871 1.234 1.579l-1.02 1.566c-.757 1.163-1.882 3.017-2.837 4.338-1.191 1.649-1.81 1.817-2.486 1.817-.524 0-1.038-.237-1.383-.794-.263-.426-.464-1.13-.464-2.046 0-2.221.63-4.535 1.66-6.088.454-.687.964-1.226 1.533-1.533a2.264 2.264 0 0 1 1.088-.285z"
											fill="currentColor"
										/>
									</svg>
									<span class="sr-only">Login with Meta</span>
								</Button>
							</Field> -->

							<!-- {#if setupRequired} -->
							<FieldDescription class="text-center">
								{t('auth.login.no_account')} <a href="/register">{t('auth.login.sign_up')}</a>
							</FieldDescription>
							<!-- {/if} -->
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
			{#if isCloud}
				{t('auth.login.continue_terms')} <a href="/terms">{t('auth.login.terms_of_service')}</a>,
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
