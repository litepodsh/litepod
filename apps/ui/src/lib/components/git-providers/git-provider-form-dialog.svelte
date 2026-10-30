<script lang="ts">
	import * as Dialog from '#lib/components/ui/dialog/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Label } from '#lib/components/ui/label/index.js';
	import * as Select from '#lib/components/ui/select/index.js';
	import GitProviderIcon from '#lib/components/custom/git-provider-icon.svelte';
	import CircleCheckIcon from '@lucide/svelte/icons/circle-check';
	import LoaderCircleIcon from '@lucide/svelte/icons/loader-circle';
	import {
		testGitProviderRepositories,
		type GitProviderDto,
		type GitProviderInput,
		type GitRepositoryDto,
	} from '#lib/clients/git-provider-client.js';
	import { t } from '#lib/i18n/index.svelte.js';
	import { toast } from '#lib/utils/toast.js';

	// Gitea and Forgejo issue OAuth2 tokens (sent as basic auth `oauth2:<token>`) as well as
	// personal access tokens (sent in the `Authorization: token` header).
	const AUTH_METHOD_TYPES = ['gitea', 'forgejo'];
	const AUTH_METHODS = [
		{ value: 'gitea_oauth', label: 'git-providers.form-dialog.auth_gitea_oauth' },
		{ value: 'oauth2', label: 'git-providers.form-dialog.auth_oauth2' },
		{ value: 'token', label: 'git-providers.form-dialog.auth_token' },
	] as const;

	const PROVIDER_TYPES = [
		{
			value: 'github',
			label: 'git-providers.form-dialog.type_github',
			host: 'https://github.com',
			hint: 'git-providers.form-dialog.type_github_hint',
		},
		{
			value: 'gitlab',
			label: 'git-providers.form-dialog.type_gitlab',
			host: 'https://gitlab.com',
			hint: 'git-providers.form-dialog.type_gitlab_hint',
		},
		{
			value: 'bitbucket',
			label: 'git-providers.form-dialog.type_bitbucket',
			host: 'https://bitbucket.org',
			hint: 'git-providers.form-dialog.type_bitbucket_hint',
		},
		{
			value: 'gitea',
			label: 'git-providers.form-dialog.type_gitea',
			host: '',
			hint: 'git-providers.form-dialog.type_gitea_hint',
		},
		{
			value: 'forgejo',
			label: 'git-providers.form-dialog.type_forgejo',
			host: '',
			hint: 'git-providers.form-dialog.type_forgejo_hint',
		},
		{
			value: 'generic',
			label: 'git-providers.form-dialog.type_generic',
			host: '',
			hint: 'git-providers.form-dialog.type_generic_hint',
		},
	] as const;

	let {
		open = $bindable(false),
		provider = null,
		organizationId,
		onSubmit,
	}: {
		open?: boolean;
		provider?: GitProviderDto | null;
		organizationId: string;
		onSubmit: (input: GitProviderInput) => Promise<void>;
	} = $props();

	let providerType = $state<string>('github');
	let authMethod = $state<string>('token');
	let name = $state('');
	let hostUrl = $state('');
	let username = $state('');
	let password = $state('');
	let saving = $state(false);
	let testing = $state(false);
	let testError = $state<string | null>(null);
	let testedRepositories = $state<GitRepositoryDto[] | null>(null);
	let testRepoUrl = $state('');
	let testRequestVersion = $state(0);
	let wasOpen = $state(false);

	let activeType = $derived(
		PROVIDER_TYPES.find((type) => type.value === providerType) ?? PROVIDER_TYPES[0],
	);

	$effect(() => {
		if (open && !wasOpen) {
			providerType = provider?.provider_type ?? 'github';
			authMethod = provider?.auth_method ?? defaultAuthMethod(provider?.provider_type ?? 'github');
			name = provider?.name ?? '';
			hostUrl =
				provider?.host_url ??
				PROVIDER_TYPES.find((type) => type.value === (provider?.provider_type ?? 'github'))!.host;
			username = provider?.username ?? '';
			password = '';
			testRepoUrl = '';
			clearTest();
		}
		wasOpen = open;
	});

	// Gitea/Forgejo default to the OAuth flow; everything else uses its token header.
	function defaultAuthMethod(type: string) {
		return AUTH_METHOD_TYPES.includes(type) ? 'gitea_oauth' : 'token';
	}

	// The OAuth app is created after the provider (its redirect URI embeds the provider id),
	// so this method collects no credentials here — the provider card finishes the flow.
	let isOauth = $derived(authMethod === 'gitea_oauth');

	let showAuthMethod = $derived(AUTH_METHOD_TYPES.includes(providerType));
	let activeAuthMethod = $derived(
		AUTH_METHODS.find((method) => method.value === authMethod) ?? AUTH_METHODS[1],
	);

	function selectAuthMethod(value: string) {
		authMethod = value;
		clearTest();
	}

	function selectType(value: string) {
		providerType = value;
		authMethod = defaultAuthMethod(value);
		const preset = PROVIDER_TYPES.find((type) => type.value === value);
		if (preset?.host) hostUrl = preset.host;
		clearTest();
	}

	function clearTest() {
		testRequestVersion += 1;
		testError = null;
		testedRepositories = null;
	}

	// Generic servers have no repository API — credentials are proven with ls-remote against one repo.
	let canTest = $derived(
		Boolean(username.trim()) &&
			Boolean(password) &&
			(providerType === 'generic' ? Boolean(testRepoUrl.trim()) : Boolean(hostUrl.trim())),
	);

	async function testConnection() {
		if (!canTest || testing) return;
		testing = true;
		clearTest();
		const requestVersion = testRequestVersion;
		try {
			const repositories = await testGitProviderRepositories(organizationId, {
				provider_type: providerType,
				host_url: hostUrl.trim(),
				username: username.trim(),
				password,
				auth_method: authMethod,
				...(providerType === 'generic' ? { repo_url: testRepoUrl.trim() } : {}),
			});
			if (requestVersion === testRequestVersion) testedRepositories = repositories;
		} catch (error) {
			if (requestVersion === testRequestVersion)
				testError =
					error instanceof Error ? error.message : t('git-providers.form-dialog.test_failed');
		} finally {
			testing = false;
		}
	}

	async function submit() {
		if (!name.trim()) {
			toast.error(t('git-providers.form-dialog.name_required'));
			return;
		}
		if (!isOauth && !username.trim()) {
			toast.error(t('git-providers.form-dialog.username_required'));
			return;
		}
		if (!isOauth && !provider && !password) {
			toast.error(t('git-providers.form-dialog.password_required'));
			return;
		}
		saving = true;
		try {
			await onSubmit({
				name: name.trim(),
				provider_type: providerType,
				host_url: hostUrl.trim(),
				username: username.trim() || name.trim(),
				auth_method: authMethod,
				// Stored password is unused by `gitea_oauth` but the column is NOT NULL.
				...(isOauth ? (provider ? {} : { password: 'oauth' }) : password ? { password } : {}),
			});
			open = false;
		} catch (error) {
			toast.error(
				error instanceof Error ? error.message : t('git-providers.form-dialog.save_failed'),
			);
		} finally {
			saving = false;
		}
	}
</script>

<Dialog.Root bind:open>
	<Dialog.Content
		class="flex max-h-[90vh] flex-col overflow-hidden sm:max-w-lg"
		onOpenAutoFocus={(event) => {
			if (!provider) event.preventDefault();
		}}>
		<Dialog.Header class="shrink-0">
			<Dialog.Title
				>{provider
					? t('git-providers.form-dialog.title_edit')
					: t('git-providers.form-dialog.title_add')}</Dialog.Title>
			<Dialog.Description>{t('git-providers.form-dialog.description')}</Dialog.Description>
		</Dialog.Header>

		<form
			id="git-provider-form"
			class="grid min-h-0 flex-1 auto-rows-min gap-4 overflow-y-auto p-6 pt-0"
			onsubmit={(event) => {
				event.preventDefault();
				void submit();
			}}>
			<div class="grid gap-2">
				<Label>{t('git-providers.form-dialog.provider_type')}</Label>
				<Select.Root type="single" value={providerType} onValueChange={selectType}>
					<Select.Trigger class="w-full" aria-label={t('git-providers.form-dialog.provider_type')}>
						<span class="flex items-center gap-2"
							><GitProviderIcon type={providerType} class="size-4" />{t(activeType.label)}</span>
					</Select.Trigger>
					<Select.Content>
						{#each PROVIDER_TYPES as type (type.value)}
							<Select.Item value={type.value} label={t(type.label)}>
								<span class="flex items-center gap-2"
									><GitProviderIcon type={type.value} class="size-4" />{t(type.label)}</span>
							</Select.Item>
						{/each}
					</Select.Content>
				</Select.Root>
				<p class="text-xs text-muted-foreground">{t(activeType.hint)}</p>
			</div>
			{#if showAuthMethod}
				<div class="grid gap-2">
					<Label>{t('git-providers.form-dialog.authentication')}</Label>
					<Select.Root type="single" value={authMethod} onValueChange={selectAuthMethod}>
						<Select.Trigger class="w-full" aria-label={t('git-providers.form-dialog.auth_method')}
							>{t(activeAuthMethod.label)}</Select.Trigger>
						<Select.Content>
							{#each AUTH_METHODS as method (method.value)}
								<Select.Item value={method.value} label={t(method.label)} />
							{/each}
						</Select.Content>
					</Select.Root>
					<p class="text-xs text-muted-foreground">
						{authMethod === 'gitea_oauth'
							? t('git-providers.form-dialog.auth_hint_gitea_oauth')
							: authMethod === 'oauth2'
								? t('git-providers.form-dialog.auth_hint_oauth2')
								: t('git-providers.form-dialog.auth_hint_token')}
					</p>
				</div>
			{/if}
			<div class="grid gap-2">
				<Label for="provider-name">{t('git-providers.form-dialog.name')}</Label>
				<Input
					id="provider-name"
					bind:value={name}
					maxlength={100}
					placeholder={t('git-providers.form-dialog.name_placeholder')}
					autofocus />
			</div>
			<div class="grid gap-2">
				<Label for="provider-host"
					>{t('git-providers.form-dialog.host')}
					<span class="text-muted-foreground">{t('git-providers.form-dialog.host_optional')}</span
					></Label>
				<Input
					id="provider-host"
					bind:value={hostUrl}
					oninput={clearTest}
					placeholder="https://github.com" />
			</div>
			{#if !isOauth}
			<div class="grid gap-2">
				<Label for="provider-username">{t('git-providers.form-dialog.username')}</Label>
				<Input
					id="provider-username"
					bind:value={username}
					oninput={clearTest}
					maxlength={200}
					placeholder="octocat" />
			</div>
			<div class="grid gap-2">
				<Label for="provider-password"
					>{t('git-providers.form-dialog.password')}
					{#if provider}<span class="text-muted-foreground"
							>{t('git-providers.form-dialog.password_keep')}</span
						>{/if}</Label>
				<Input id="provider-password" type="password" bind:value={password} oninput={clearTest} />
			</div>
			<section
				class="grid gap-3 rounded-lg border bg-muted/30 p-3"
				aria-label={t('git-providers.form-dialog.test_title')}>
				<div>
					<h3 class="text-sm font-medium">{t('git-providers.form-dialog.test_title')}</h3>
					{#if providerType === 'generic'}
						<p class="mt-1 text-xs text-muted-foreground">
							{t('git-providers.form-dialog.test_hint_generic')}
						</p>
					{:else}
						<p class="mt-1 text-xs text-muted-foreground">
							{t('git-providers.form-dialog.test_hint')}
						</p>
					{/if}
				</div>
				{#if providerType === 'generic'}
					<div class="grid gap-2">
						<Label for="provider-test-repo">{t('git-providers.form-dialog.test_repo_label')}</Label>
						<Input
							id="provider-test-repo"
							bind:value={testRepoUrl}
							oninput={clearTest}
							placeholder="https://git.example.com/organization/app.git" />
					</div>
				{/if}
				<div>
					<div class="flex flex-wrap items-center gap-2">
						<Button
							type="button"
							variant="outline"
							onclick={() => void testConnection()}
							disabled={!canTest || testing}>
							{#if testing}<LoaderCircleIcon class="size-4 animate-spin" />
								{t('git-providers.form-dialog.testing')}{:else}{t(
									'git-providers.form-dialog.test',
								)}{/if}
						</Button>
						{#if !password}
							<span class="text-xs text-muted-foreground"
								>{t('git-providers.form-dialog.test_needs_password')}</span>
						{/if}
					</div>
					{#if testError}
						<p
							class="rounded-md border border-destructive/30 bg-destructive/5 p-2 text-xs text-destructive">
							{testError}
						</p>
					{:else if testedRepositories !== null}
						<div class="grid gap-2" aria-live="polite">
							<p class="flex items-center gap-1 text-xs font-medium text-success">
								<CircleCheckIcon class="size-4" />
								{providerType === 'generic'
									? t('git-providers.form-dialog.test_success_generic')
									: testedRepositories.length === 1
										? t('git-providers.form-dialog.test_success_one', {
												count: testedRepositories.length,
											})
										: t('git-providers.form-dialog.test_success_many', {
												count: testedRepositories.length,
											})}
							</p>
							{#if testedRepositories.length > 0}
								<ul class="max-h-52 space-y-2 overflow-y-auto rounded-md border bg-background p-2">
									{#each testedRepositories as repository (repository.clone_url)}
										<li class="grid gap-1 border-b pb-2 last:border-0 last:pb-0">
											<div class="flex items-center gap-2">
												<span class="min-w-0 truncate text-sm font-medium"
													>{repository.full_name}</span
												><span class="rounded-full bg-muted px-1.5 py-0.5 text-[10px]"
													>{repository.private
														? t('git-providers.form-dialog.repo_private')
														: t('git-providers.form-dialog.repo_public')}</span
												>{#if repository.default_branch}<span class="text-xs text-muted-foreground"
														>{repository.default_branch}</span
													>{/if}
											</div>
											<code class="truncate text-[11px] text-muted-foreground"
												>{repository.clone_url}</code>
										</li>
									{/each}
								</ul>
							{:else}
								<p class="text-xs text-muted-foreground">
									{t('git-providers.form-dialog.test_no_repos')}
								</p>
							{/if}
						</div>
					{/if}
				</div>
			</section>
			{/if}
		</form>
		<Dialog.Footer class="shrink-0 border-t border-border bg-popover/50 px-6 py-4">
			<Button type="button" variant="outline" onclick={() => (open = false)}
				>{t('common.common.cancel')}</Button>
			<Button type="submit" form="git-provider-form" disabled={saving}
				>{saving
					? t('git-providers.form-dialog.saving')
					: provider
						? t('git-providers.form-dialog.save_changes')
						: t('git-providers.form-dialog.add_provider')}</Button>
		</Dialog.Footer>
	</Dialog.Content>
</Dialog.Root>
