<script lang="ts">
	import { goto } from '$app/navigation';
	import { onMount, tick, untrack } from 'svelte';
	import LitepodMark from '#lib/components/custom/litepod-mark.svelte';
	import { branding, brandName } from '#lib/ee/branding/branding.svelte.js';
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
	import ShieldCheckIcon from '@lucide/svelte/icons/shield-check';
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

	// When the tab regains focus (user returns from their authenticator app) or becomes
	// visible again, look for a freshly copied code.
	onMount(() => {
		window.addEventListener('focus', detectClipboardCode);
		const onVisibility = () => {
			if (document.visibilityState === 'visible') detectClipboardCode();
		};
		document.addEventListener('visibilitychange', onVisibility);
		return () => {
			window.removeEventListener('focus', detectClipboardCode);
			document.removeEventListener('visibilitychange', onVisibility);
		};
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
				// The code may already be on the clipboard from the authenticator app.
				detectClipboardCode();
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

	function handleTotpSubmit(e: SubmitEvent) {
		e.preventDefault();
		void submitTotp();
	}

	async function submitTotp() {
		if (!totpChallenge || loading) return;
		// Accept the six-digit TOTP or an eight-character recovery code; ignore
		// anything else the user may have typed or pasted.
		const code = totpCode.toUpperCase().replace(/[^A-Z0-9]/g, '').slice(0, 8);
		if (!code) return;
		error = '';
		loading = true;
		try {
			await completeTotpLogin(totpChallenge, code);
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
		} finally {
			loading = false;
			// Re-focus only after the input is re-enabled so a failed code can be
			// retyped immediately. On the 401 path the form unmounts and this no-ops.
			await tick();
			totpInput?.focus();
		}
	}

	/** True when the field holds a complete code: six TOTP digits or eight recovery chars. */
	function isCompleteCode(code: string) {
		return /^\d{6}$/.test(code) || /^[A-Z0-9]{8}$/.test(code);
	}

	// Keep the code field to alphanumerics, uppercase, and at most eight characters.
	// A complete TOTP (six digits) or recovery code (eight chars) submits itself.
	$effect(() => {
		const cleaned = totpCode.toUpperCase().replace(/[^A-Z0-9]/g, '').slice(0, 8);
		if (cleaned !== totpCode) {
			totpCode = cleaned;
		} else if (isCompleteCode(cleaned)) {
			// untrack: otherwise `loading`/`totpChallenge` become deps and a success re-submits the spent challenge (401).
			untrack(() => void submitTotp());
		}
	});

	function maskEmail(address: string) {
		const [user, domain] = address.split('@');
		if (!user || !domain) return address;
		const visible = user.slice(0, 1);
		return `${visible}${'•'.repeat(Math.max(0, user.length - visible.length))}@${domain}`;
	}

	// Auto-detect a code copied from an authenticator app (Proton Pass, 1Password…)
	// when the page regains focus, then fill it and let the effect submit it. Clipboard
	// read is permission-gated, so this is best-effort and no-ops where the browser refuses.
	function detectClipboardCode() {
		const clipboard = navigator.clipboard;
		if (!totpChallenge || loading || totpCode || !clipboard) return;
		clipboard
			.readText()
			.then((text) => {
				const code = text.trim().toUpperCase().replace(/[^A-Z0-9]/g, '').slice(0, 8);
				if (totpChallenge && !loading && !totpCode && isCompleteCode(code)) totpCode = code;
			})
			.catch(() => {
				// Fall back to manual entry.
			});
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

<div class={cn('bg-background grid min-h-svh lg:grid-cols-2', className)} {...restProps}>
	<div class="flex items-center justify-center p-6 md:p-10">
		<div class="flex w-full max-w-sm flex-col gap-6">
			{#if branding.logo_url}
				<img src={branding.logo_url} alt={brandName()} class="size-8 object-contain" />
			{:else}
				<LitepodMark class="size-8" />
			{/if}
			{#if totpChallenge}
				<form onsubmit={handleTotpSubmit}>
					<FieldGroup>
						<div class="flex flex-col gap-1">
							<div class="flex items-center gap-2">
								<ShieldCheckIcon class="text-muted-foreground size-5 shrink-0" />
								<h1 class="text-2xl font-semibold tracking-tight">
									{t('auth.login.two_factor_authentication')}
								</h1>
							</div>
							<p class="text-muted-foreground text-sm">
								{t('auth.login.two_factor_description')}
							</p>
							<p class="text-muted-foreground mt-1 text-sm font-medium">
								{t('auth.login.continue_as', { email: maskEmail(email) })}
							</p>
						</div>

						{#if error}
							<p class="text-destructive text-sm font-medium" role="alert">{error}</p>
						{/if}

						<Field>
							<FieldLabel for="totp-{id}" class="sr-only">{t('auth.login.code')}</FieldLabel>
							<!-- svelte-ignore a11y_autofocus -->
							<Input
								bind:ref={totpInput}
								id="totp-{id}"
								class="rounded-sm h-14 text-center font-mono text-2xl tracking-[0.3em] placeholder:tracking-normal placeholder:text-base"
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
							<Button
								type="submit"
								class="w-full rounded-sm"
								disabled={loading || !totpCode.trim()}>
								{loading ? t('auth.login.verifying') : t('auth.login.verify')}
							</Button>
						</Field>

						<FieldDescription>
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
							<FieldDescription>
								{t('auth.login.support_contact')}
								<a href="mailto:info@litepod.sh">info@litepod.sh</a>
							</FieldDescription>
						{/if}
					</FieldGroup>
				</form>
			{:else}
				<form onsubmit={handleSubmit}>
					<FieldGroup>
						<div class="flex flex-col gap-1">
							<h1 class="text-2xl font-semibold tracking-tight">
								{t('auth.login.log_in_to', { name: branding.name ?? 'Litepod' })}
							</h1>
							<FieldDescription>
								{t('auth.login.no_account')} <a href="/register">{t('auth.login.sign_up')}</a>
							</FieldDescription>
						</div>

						{#if error}
							<p class="text-destructive text-sm font-medium" role="alert">{error}</p>
						{/if}

						<Field>
							<FieldLabel for="email-{id}">{t('auth.login.email')}</FieldLabel>
							<Input
								id="email-{id}"
								class="rounded-sm"
								type="email"
								autocomplete="username"
								placeholder={t('auth.login.email_placeholder')}
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
									class="rounded-sm pr-10"
									bind:value={password}
									oninput={() => revalidateField('password')} />
								<Button
									type="button"
									variant="ghost"
									size="icon"
									class="absolute right-1 top-1/2 size-7 -translate-y-1/2 rounded-sm"
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
							<Button type="submit" class="w-full rounded-sm" disabled={loading}>
								{loading ? t('auth.login.logging_in') : t('auth.login.login')}
							</Button>
						</Field>
						{#if ssoAvailable}
							<FieldSeparator class="*:data-[slot=field-separator-content]:bg-background">
								{t('auth.login.or')}
							</FieldSeparator>
							<Field>
								<Button
									type="button"
									variant="outline"
									class="w-full rounded-sm"
									disabled={loading || ssoLoading}
									onclick={handleSso}>
									{ssoLoading ? t('auth.login.sso_redirecting') : t('auth.login.continue_with_sso')}
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
					</FieldGroup>
				</form>
			{/if}
			<FieldDescription>
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
	<div
		class={cn(
			'relative hidden flex-col justify-end overflow-hidden p-10 lg:flex',
			// A custom logo gets a neutral field: it may not be legible on the brand color.
			branding.logo_url
				? 'bg-muted'
				: 'bg-primary text-primary-foreground bg-[linear-gradient(160deg,color-mix(in_oklch,var(--primary)_94%,white),var(--primary))]',
		)}>
		{#if branding.logo_url}
			<img
				src={branding.logo_url}
				alt=""
				class="absolute top-1/2 left-1/2 size-48 -translate-x-1/2 -translate-y-1/2 object-contain" />
		{:else}
			<!-- The logo's own pill at -24deg, cropped by the panel. -->
			<span
				aria-hidden="true"
				class="absolute top-1/2 -right-24 h-75 w-110 -translate-y-[58%] -rotate-24 rounded-full bg-current">
			</span>
		{/if}
		{#if !branding.name}
			<p class="relative max-w-[12ch] text-4xl leading-tight font-semibold tracking-tight">
				{t('auth.login.tagline')}
			</p>
		{/if}
	</div>
</div>
