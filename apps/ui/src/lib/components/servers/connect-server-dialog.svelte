<script lang="ts">
	import CheckIcon from '@lucide/svelte/icons/check';
	import LoaderCircleIcon from '@lucide/svelte/icons/loader-circle';
	import ShieldAlertIcon from '@lucide/svelte/icons/shield-alert';
	import TerminalIcon from '@lucide/svelte/icons/square-terminal';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import * as Dialog from '#lib/components/ui/dialog/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Label } from '#lib/components/ui/label/index.js';
	import * as Select from '#lib/components/ui/select/index.js';
	import { Textarea } from '#lib/components/ui/textarea/index.js';
	import AnimatedTabs from '#lib/components/custom/animated-tabs/animated-tabs.svelte';
	import CopyButton from '#lib/components/custom/copy-button.svelte';
	import ServerJobLog from './server-job-log.svelte';
	import ServerSwapNotice from './server-swap-notice.svelte';
	import {
		confirmHostKey,
		createServer,
		getServerPublicKey,
		healthcheckServer,
		preflightServer,
		setupServer,
		type IngressMode,
		type PodmanMode,
		type Server,
		type ServerJob,
	} from '#lib/clients/server-client.js';
	import { toast } from '#lib/utils/toast.js';
	import { t } from '#lib/i18n/index.svelte.js';
	import { untrack } from 'svelte';

	let {
		open = $bindable(false),
		organizationId,
		cloudMode,
		isSysadmin,
		resume = null,
		resumeStep = null,
		onDone,
	}: {
		open?: boolean;
		/** The organization that will own the server this wizard registers. */
		organizationId: string;
		/** Cloud remote servers always expose their own public Caddy edge. */
		cloudMode: boolean;
		/** The installation owner is exempt from Litepod Cloud Billing. */
		isSysadmin: boolean;
		/** Reopens the wizard on an already-registered server at the step its state implies. */
		resume?: Server | null;
		/** Overrides that step, e.g. to jump straight back to the public key. */
		resumeStep?: number | null;
		onDone: () => void | Promise<void>;
	} = $props();

	type RemoteIngress = Exclude<IngressMode, 'central'>;

	const STEPS = [
		{ title: t('servers.connect.step_remote_title'), hint: t('servers.connect.step_remote_hint') },
		{ title: t('servers.connect.step_key_title'), hint: t('servers.connect.step_key_hint') },
		{
			title: t('servers.connect.step_authorize_title'),
			hint: t('servers.connect.step_authorize_hint'),
		},
		{ title: t('servers.connect.step_verify_title'), hint: t('servers.connect.step_verify_hint') },
		{
			title: t('servers.connect.step_install_title'),
			hint: t('servers.connect.step_install_hint'),
		},
	];

	// Labels are kept the same length: the animated pill sizes to its trigger, so a long
	// "(recommended)" suffix would make one tab visibly wider than the other.
	const PODMAN_MODES = [
		{ value: 'rootless', label: t('servers.connect.podman_rootless') },
		{ value: 'rootful', label: t('servers.connect.podman_rootful') },
	];

	const INGRESS_OPTIONS: { value: RemoteIngress; label: string; hint: string }[] = [
		{
			value: 'private_relay',
			label: t('servers.connect.ingress_private_relay'),
			hint: t('servers.connect.ingress_private_relay_hint'),
		},
		{
			value: 'public_edge',
			label: t('servers.connect.ingress_public_edge'),
			hint: t('servers.connect.ingress_public_edge_hint'),
		},
		{
			value: 'private_only',
			label: t('servers.connect.ingress_private_only'),
			hint: t('servers.connect.ingress_private_only_hint'),
		},
	];

	let step = $state(0);
	let serverId = $state<string | null>(null);
	let server = $state<Server | null>(null);
	let busy = $state(false);
	let stepError = $state('');

	// Step 1
	let name = $state('');
	let host = $state('');
	let port = $state('22');
	let user = $state('litepod');
	let ingress = $state<RemoteIngress>('private_relay');
	let podmanMode = $state<PodmanMode>('rootless');
	let publicAddress = $state('');

	// Step 2
	let keySource = $state<'generate' | 'import'>('generate');
	let privateKey = $state('');

	// Step 3
	let publicKey = $state('');
	let authorizedKeysCommand = $state('');

	// Step 4
	let fingerprint = $state<string | null>(null);

	// Step 5
	let job = $state<ServerJob | null>(null);

	const selectedIngress = $derived(INGRESS_OPTIONS.find((option) => option.value === ingress)!);

	// Setup fails late and confusingly when a firewall blocks the outbound package fetch or the
	// inbound ACME challenge, so the requirements are stated before the server is registered.
	const firewallRules = $derived([
		t('servers.connect.fw_ssh', { port: port || '22' }),
		t('servers.connect.fw_outbound'),
		...(ingress === 'public_edge' ? [t('servers.connect.fw_tls')] : []),
	]);
	// Most VPS providers put a cloud firewall in front of the instance, so a ufw rule alone is not
	// enough there. The SSH allow rule comes before `ufw enable` on purpose: enabling first would
	// drop the session that is typing these commands.
	const ufwCommands = $derived([
		`sudo ufw allow ${port || '22'}/tcp`,
		...(ingress === 'public_edge' ? ['sudo ufw allow 80/tcp', 'sudo ufw allow 443/tcp'] : []),
		'sudo ufw enable',
		'sudo ufw status verbose',
	]);
	const isRoot = $derived(user.trim() === 'root');
	const finished = $derived(server?.state === 'ready');

	// Rootless Podman cannot run as root, so typing "root" as the SSH user flips the mode instead of
	// letting the wizard reach validation with a combination the API rejects. The flip is remembered
	// so switching back to an unprivileged user restores the recommended mode.
	let rootfulForcedByUser = $state(false);
	$effect(() => {
		const rootUser = user.trim() === 'root';
		untrack(() => {
			if (rootUser && podmanMode === 'rootless') {
				podmanMode = 'rootful';
				rootfulForcedByUser = true;
			} else if (!rootUser && rootfulForcedByUser) {
				podmanMode = 'rootless';
				rootfulForcedByUser = false;
			}
		});
	});

	function reset() {
		step = 0;
		serverId = null;
		server = null;
		busy = false;
		stepError = '';
		name = '';
		host = '';
		port = '22';
		user = 'litepod';
		ingress = cloudMode ? 'public_edge' : 'private_relay';
		podmanMode = 'rootless';
		publicAddress = '';
		rootfulForcedByUser = false;
		keySource = 'generate';
		privateKey = '';
		publicKey = '';
		authorizedKeysCommand = '';
		fingerprint = null;
		job = null;
	}

	/** Where an already-registered server picks the flow back up. */
	function stepForState(target: Server): number {
		switch (target.state) {
			case 'needs_verification':
				return target.host_key_fingerprint ? 3 : 2;
			case 'needs_setup':
			case 'setting_up':
				return 4;
			default:
				return 3;
		}
	}

	let wasOpen = false;
	$effect(() => {
		if (open === wasOpen) return;
		wasOpen = open;
		if (!open) return;
		reset();
		if (!resume) return;
		server = resume;
		serverId = resume.id;
		name = resume.name;
		host = resume.ssh_host ?? '';
		port = String(resume.ssh_port ?? 22);
		user = resume.ssh_user ?? '';
		ingress = cloudMode
			? 'public_edge'
			: resume.ingress_mode === 'central'
				? 'private_relay'
				: resume.ingress_mode;
		podmanMode = resume.podman_mode;
		publicAddress = resume.public_address ?? '';
		step = resumeStep ?? stepForState(resume);
		if (step === 2) void loadPublicKey(resume.id);
	});

	function detailsError(): string | null {
		const trimmedName = name.trim();
		if (!trimmedName || trimmedName.length > 100) return t('servers.connect.name_invalid_len');
		const trimmedHost = host.trim();
		const isIp = /^[0-9.]+$/.test(trimmedHost) || trimmedHost.includes(':');
		const isHostname = /^(?!-)[A-Za-z0-9-]{1,63}(?<!-)(\.(?!-)[A-Za-z0-9-]{1,63}(?<!-))*$/.test(
			trimmedHost,
		);
		if (!trimmedHost || (!isIp && !isHostname)) return t('servers.connect.host_invalid');
		const parsedPort = Number(port);
		if (!Number.isInteger(parsedPort) || parsedPort < 1 || parsedPort > 65535)
			return t('servers.connect.port_invalid');
		if (!/^[A-Za-z0-9_.-]{1,32}$/.test(user.trim())) return t('servers.connect.user_invalid');
		if (podmanMode === 'rootless' && user.trim() === 'root')
			return t('servers.connect.rootless_user_invalid');
		if (ingress === 'public_edge' && !publicAddress.trim())
			return t('servers.connect.public_edge_address_required');
		return null;
	}

	function keyError(): string | null {
		if (keySource !== 'import') return null;
		if (!privateKey.includes('-----BEGIN OPENSSH PRIVATE KEY-----'))
			return t('servers.connect.import_key_invalid');
		if (privateKey.toUpperCase().includes('ENCRYPTED'))
			return t('servers.connect.encrypted_key_unsupported');
		return null;
	}

	/** Step 1 only checks the connection details; the key is chosen on step 2. */
	function continueToKey() {
		const invalid = detailsError();
		if (invalid) {
			stepError = invalid;
			return;
		}
		stepError = '';
		step = 1;
	}

	async function loadPublicKey(id: string) {
		const key = await getServerPublicKey(organizationId, id);
		publicKey = key.public_key;
		authorizedKeysCommand = key.authorized_keys_command;
	}

	async function register() {
		const invalid = detailsError() ?? keyError();
		if (invalid) {
			stepError = invalid;
			return;
		}
		await doRegister();
	}

	async function doRegister() {
		busy = true;
		stepError = '';
		try {
			const registration = await createServer(organizationId, {
				name: name.trim(),
				host: host.trim(),
				port: Number(port),
				user: user.trim(),
				ingress_mode: ingress,
				podman_mode: podmanMode,
				public_address: ingress === 'public_edge' ? publicAddress.trim() : undefined,
				credential:
					keySource === 'import'
						? { kind: 'import', private_key: privateKey }
						: { kind: 'generate' },
			});
			server = registration.server;
			serverId = registration.server.id;
			privateKey = '';
			await loadPublicKey(registration.server.id);
			step = 2;
		} catch (caught) {
			stepError = caught instanceof Error ? caught.message : t('servers.connect.register_failed');
		} finally {
			busy = false;
		}
	}

	/** Scans the host key on first contact, then collects facts once it is trusted. */
	async function testConnection() {
		if (!serverId) return;
		busy = true;
		stepError = '';
		try {
			const result = await preflightServer(organizationId, serverId);
			server = result.server;
			if (result.observed_host_key_fingerprint) {
				fingerprint = result.observed_host_key_fingerprint;
				step = 3;
				return;
			}
			fingerprint = null;
			step = 3;
		} catch (caught) {
			stepError = caught instanceof Error ? caught.message : t('servers.connect.ssh_unreachable');
			step = 3;
		} finally {
			busy = false;
		}
	}

	async function trustFingerprint() {
		if (!serverId || !fingerprint) return;
		busy = true;
		stepError = '';
		try {
			await confirmHostKey(organizationId, serverId, fingerprint);
			fingerprint = null;
			// Facts are only reachable once the key is pinned, so preflight runs again here.
			const result = await preflightServer(organizationId, serverId);
			server = result.server;
		} catch (caught) {
			stepError = caught instanceof Error ? caught.message : t('servers.connect.confirm_failed');
		} finally {
			busy = false;
		}
	}

	async function install(repair = false) {
		if (!serverId) return;
		busy = true;
		stepError = '';
		try {
			job = await setupServer(organizationId, serverId, repair);
			step = 4;
		} catch (caught) {
			stepError =
				caught instanceof Error ? caught.message : t('servers.connect.setup_start_failed');
		} finally {
			busy = false;
		}
	}

	function close() {
		open = false;
		void onDone();
	}
</script>

{#snippet remoteCommand(command: string)}
	<div class="flex items-start gap-2 rounded-md border bg-muted/50 p-2 font-mono text-xs">
		<span class="mt-0.5 shrink-0 select-none text-muted-foreground">$</span>
		<code class="min-w-0 flex-1 break-all whitespace-pre-wrap">{command}</code>
		<CopyButton
			value={command}
			label={null}
			revealOnHover={false}
			class="mt-0.5 shrink-0 text-muted-foreground hover:text-foreground"
			errorMessage={t('servers.connect.clipboard_error')} />
	</div>
{/snippet}

{#snippet onYourServer(title: string, children: import('svelte').Snippet)}
	<div class="space-y-2 rounded-lg border border-dashed p-3">
		<p class="flex items-center gap-2 text-sm font-medium">
			<TerminalIcon class="size-4 text-muted-foreground" />
			{title}
		</p>
		{@render children()}
	</div>
{/snippet}

<Dialog.Root bind:open>
	<!-- Flex column instead of the default grid: the step body is the only part allowed to grow, so
	     the header and footer stay visible on short viewports and the content scrolls instead. -->
	<Dialog.Content class="flex max-h-[85vh] flex-col gap-0 overflow-hidden p-0 sm:max-w-3xl">
		<Dialog.Header class="shrink-0 border-b px-6 py-4">
			<Dialog.Title>{t('organizations.servers.connect')}</Dialog.Title>
			<Dialog.Description>
				{t('servers.connect.description')}
			</Dialog.Description>
		</Dialog.Header>

		<div class="grid min-h-0 flex-1 grid-cols-1 overflow-hidden sm:grid-cols-[13rem_1fr]">
			<!-- Progress rail. Completed steps stay clickable-looking but are not links: going
			     back would desynchronize the remote server's real state from the wizard. -->
			<ol class="hidden gap-1 border-r bg-muted/30 p-4 sm:grid sm:content-start">
				{#each STEPS as entry, index (entry.title)}
					<li
						class="flex items-start gap-3 rounded-md p-2 {index === step
							? 'bg-background shadow-sm'
							: ''}">
						<span
							class="mt-0.5 flex size-5 shrink-0 items-center justify-center rounded-full border text-[11px] font-medium
								{index < step ? 'border-success/40 bg-success/10 text-success' : ''}
								{index === step ? 'border-primary bg-primary text-primary-foreground' : ''}
								{index > step ? 'text-muted-foreground' : ''}">
							{#if index < step}<CheckIcon class="size-3" />{:else}{index + 1}{/if}
						</span>
						<span class="min-w-0">
							<span
								class="block text-sm leading-tight font-medium {index > step
									? 'text-muted-foreground'
									: ''}">{entry.title}</span>
							<span class="block text-xs leading-tight text-muted-foreground">{entry.hint}</span>
						</span>
					</li>
				{/each}
			</ol>

			<div class="min-h-0 overflow-y-auto px-6 py-5">
				{#if stepError}
					<p
						class="mb-4 rounded-lg border border-destructive/30 bg-destructive/5 p-3 text-sm text-destructive">
						{stepError}
					</p>
				{/if}

				{#if step === 0}
					<div class="space-y-5">
						<div class="grid gap-4 sm:grid-cols-2">
							<div class="grid gap-2">
								<Label for="server-name">{t('organizations.common.name')}</Label>
								<Input
									id="server-name"
									bind:value={name}
									maxlength={100}
									placeholder="web-01"
									autocomplete="off" />
							</div>
							<div class="grid gap-2">
								<Label for="server-host">{t('servers.connect.label_ssh_host')}</Label>
								<Input
									id="server-host"
									bind:value={host}
									placeholder="203.0.113.10"
									autocomplete="off" />
							</div>
							<div class="grid gap-2">
								<Label for="server-port">{t('servers.connect.label_ssh_port')}</Label>
								<Input id="server-port" bind:value={port} inputmode="numeric" placeholder="22" />
							</div>
							<div class="grid gap-2">
								<Label for="server-user">{t('servers.connect.label_ssh_user')}</Label>
								<Input
									id="server-user"
									bind:value={user}
									placeholder="litepod"
									autocomplete="off" />
							</div>
						</div>

						<div class="grid gap-2">
							<Label>{t('servers.connect.label_podman_mode')}</Label>
							<AnimatedTabs
								items={PODMAN_MODES}
								value={podmanMode}
								onValueChange={(next) => (podmanMode = next as PodmanMode)} />
							<p class="text-sm text-muted-foreground">
								{#if podmanMode === 'rootless'}
									<Badge variant="secondary" class="mr-1 align-middle"
										>{t('servers.connect.recommended')}</Badge>
									{t('servers.connect.rootless_before')}
									<code class="font-mono text-xs"
										>{user.trim() || t('servers.connect.the_ssh_user')}</code
									>{t('servers.connect.rootless_after')}
								{:else}
									{t('servers.connect.rootful_hint')}
								{/if}
							</p>

							{#if isRoot}
								<div class="rounded-md border bg-muted/40 p-3 text-xs text-muted-foreground">
									<p class="font-medium text-foreground">
										{t('servers.connect.root_switched_title')}
									</p>
									<p class="mt-1">
										{t('servers.connect.root_unavailable_before')}
										<code class="font-mono">root</code>{t('servers.connect.root_unavailable_after')}
									</p>
									<p class="mt-2">
										{t('servers.connect.rootless_switch_back')}
									</p>
									<div class="mt-2 space-y-2">
										{@render remoteCommand('sudo adduser --disabled-password --gecos "" litepod')}
										{@render remoteCommand(
											"echo 'litepod ALL=(ALL) NOPASSWD:ALL' | sudo tee /etc/sudoers.d/litepod && sudo chmod 440 /etc/sudoers.d/litepod",
										)}
									</div>
									<p class="mt-2">
										{t('servers.connect.passwordless_hint')}
									</p>
								</div>
							{/if}
						</div>

						<div class="grid gap-2">
							{#if cloudMode}
								<Label>{t('servers.connect.label_ingress')}</Label>
								<p class="rounded-md border bg-muted/40 p-3 text-sm">
									<span class="font-medium">{t('servers.connect.ingress_public_edge')}</span>{t(
										'servers.connect.cloud_public_edge_hint',
									)}
								</p>
							{:else}
								<Label for="server-ingress">{t('servers.connect.label_ingress')}</Label>
								<Select.Root type="single" bind:value={ingress}>
									<Select.Trigger id="server-ingress" class="w-full"
										>{selectedIngress.label}</Select.Trigger>
									<Select.Content>
										{#each INGRESS_OPTIONS as option (option.value)}
											<Select.Item value={option.value}>{option.label}</Select.Item>
										{/each}
									</Select.Content>
								</Select.Root>
								<p class="text-xs text-muted-foreground">{selectedIngress.hint}</p>
							{/if}

							<div class="rounded-md border bg-muted/40 p-3 text-xs text-muted-foreground">
								<p class="font-medium text-foreground">{t('servers.connect.ports_title')}</p>
								<ul class="mt-1 list-disc space-y-1 pl-4">
									{#each firewallRules as rule (rule)}
										<li>{rule}</li>
									{/each}
								</ul>
								<p class="mt-2">
									{t('servers.connect.nothing_before')}
									<code class="font-mono">127.0.0.1</code>{t('servers.connect.nothing_middle')}
									<code class="font-mono">localhost:2019</code>{t('servers.connect.nothing_after')}
								</p>

								<p class="mt-3 font-medium text-foreground">{t('servers.connect.where_title')}</p>
								<p class="mt-1">
									{t('servers.connect.vps_firewall')}
								</p>
								<p class="mt-2">
									{t('servers.connect.ufw_before')}<code class="font-mono">ufw</code>{t(
										'servers.connect.ufw_after',
									)}
								</p>
								<div class="mt-2 space-y-2">
									{#each ufwCommands as command (command)}
										{@render remoteCommand(command)}
									{/each}
								</div>
								<p class="mt-2">
									{t('servers.connect.allow_before')}<code class="font-mono">allow</code>{t(
										'servers.connect.allow_mid',
									)}<code class="font-mono">ufw enable</code>{t('servers.connect.allow_after')}
								</p>
							</div>
						</div>

						{#if ingress === 'public_edge'}
							<div class="grid gap-2">
								<Label for="server-public-address"
									>{t('servers.connect.label_public_address')}</Label>
								<Input
									id="server-public-address"
									bind:value={publicAddress}
									placeholder="203.0.113.10"
									autocomplete="off" />
								<p class="text-xs text-muted-foreground">
									{t('servers.connect.public_address_hint')}
								</p>
							</div>
						{/if}
					</div>
				{:else if step === 1}
					<div class="space-y-5">
						<div>
							<h3 class="text-sm font-medium">{t('servers.connect.step_key_title')}</h3>
							<p class="mt-1 text-sm text-muted-foreground">
								{t('servers.connect.key_intro')}
							</p>
						</div>

						<div class="grid gap-2">
							<div class="grid grid-cols-2 gap-2">
								<Button
									variant={keySource === 'generate' ? 'default' : 'outline'}
									onclick={() => (keySource = 'generate')}>
									{t('servers.connect.generate_key')}
								</Button>
								<Button
									variant={keySource === 'import' ? 'default' : 'outline'}
									onclick={() => (keySource = 'import')}>
									{t('servers.connect.use_existing_key')}
								</Button>
							</div>
							{#if keySource === 'generate'}
								<p class="text-xs text-muted-foreground">
									{t('servers.connect.generate_hint')}
								</p>
							{:else}
								<Textarea
									bind:value={privateKey}
									class="min-h-28 font-mono text-xs"
									placeholder="-----BEGIN OPENSSH PRIVATE KEY-----"
									spellcheck={false} />
								<p class="text-xs text-muted-foreground">
									{t('servers.connect.import_hint')}
								</p>
							{/if}
						</div>

						{#snippet prepareCommands()}
							{#if podmanMode === 'rootless'}
								<p class="text-xs text-muted-foreground">
									{t('servers.connect.prepare_rootless')}
								</p>
								{@render remoteCommand(
									`sudo adduser --disabled-password --gecos "" ${user.trim() || 'litepod'}`,
								)}
								{@render remoteCommand(
									`echo '${user.trim() || 'litepod'} ALL=(ALL) NOPASSWD:ALL' | sudo tee /etc/sudoers.d/${user.trim() || 'litepod'} && sudo chmod 440 /etc/sudoers.d/${user.trim() || 'litepod'}`,
								)}
							{:else if isRoot}
								<p class="text-xs text-muted-foreground">
									{t('servers.connect.prepare_root')}
								</p>
								{@render remoteCommand('grep -E "^PermitRootLogin" /etc/ssh/sshd_config')}
							{:else}
								<p class="text-xs text-muted-foreground">
									{t('servers.connect.prepare_rootful_before')}<code class="font-mono"
										>{user.trim()}</code
									>{t('servers.connect.prepare_rootful_after')}
								</p>
								{@render remoteCommand(
									`echo '${user.trim()} ALL=(ALL) NOPASSWD:ALL' | sudo tee /etc/sudoers.d/${user.trim()} && sudo chmod 440 /etc/sudoers.d/${user.trim()}`,
								)}
							{/if}
							<p class="text-xs text-muted-foreground">
								{t('servers.connect.distros_hint')}
							</p>
						{/snippet}
						{@render onYourServer(t('servers.connect.before_you_continue'), prepareCommands)}
					</div>
				{:else if step === 2}
					<div class="space-y-5">
						<div>
							<h3 class="text-sm font-medium">{t('servers.connect.install_key_title')}</h3>
							<p class="mt-1 text-sm text-muted-foreground">
								{t('servers.connect.install_key_description')}
							</p>
						</div>

						<div class="grid gap-2">
							<Label>{t('servers.connect.label_public_key')}</Label>
							<div class="rounded-md border bg-muted/50 p-3">
								<code class="block font-mono text-xs break-all"
									>{publicKey || t('organizations.common.loading')}</code>
							</div>
							{#if publicKey}
								<CopyButton
									value={publicKey}
									label={t('servers.connect.copy_public_key')}
									copiedLabel={t('common.common.copied')}
									revealOnHover={false}
									class="w-fit text-xs text-muted-foreground hover:text-foreground" />
							{/if}
						</div>

						{#snippet installKey()}
							<p class="text-xs text-muted-foreground">
								{t('servers.connect.install_log_in_before')}<code class="font-mono"
									>{user.trim()}</code
								>{t('servers.connect.install_log_in_after')}
								<code class="font-mono">authorized_keys</code>.
							</p>
							{#if authorizedKeysCommand}
								{@render remoteCommand(authorizedKeysCommand)}
							{/if}
							{#if !isRoot}
								<p class="text-xs text-muted-foreground">
									{t('servers.connect.install_root_hint')}
								</p>
								{@render remoteCommand(
									`sudo -u ${user.trim()} bash -lc ${JSON.stringify(authorizedKeysCommand)}`,
								)}
							{/if}
						{/snippet}
						{@render onYourServer(t('servers.connect.run_on_server'), installKey)}
					</div>
				{:else if step === 3}
					<div class="space-y-5">
						{#if fingerprint}
							<div class="space-y-4">
								<div
									class="flex items-start gap-3 rounded-lg border border-warning/40 bg-warning/5 p-3">
									<ShieldAlertIcon class="mt-0.5 size-4 shrink-0 text-warning" />
									<div class="min-w-0 space-y-1">
										<p class="text-sm font-medium">{t('servers.connect.verify_title')}</p>
										<p class="text-xs text-muted-foreground">
											{t('servers.connect.verify_description')}
										</p>
									</div>
								</div>

								<div class="grid gap-2">
									<Label>{t('servers.connect.label_fingerprint')}</Label>
									<div class="rounded-md border bg-muted/50 p-3">
										<code class="block font-mono text-xs break-all">{fingerprint}</code>
									</div>
								</div>

								{#snippet verifyKey()}
									<p class="text-xs text-muted-foreground">
										{t('servers.connect.verify_command_hint')}
									</p>
									{@render remoteCommand('ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub')}
								{/snippet}
								{@render onYourServer(t('servers.connect.run_on_server'), verifyKey)}
							</div>
						{:else if server?.state === 'unsupported'}
							<div
								class="rounded-lg border border-destructive/30 bg-destructive/5 p-4 text-sm text-destructive">
								<p class="font-medium">{t('servers.connect.unsupported_title')}</p>
								<p class="mt-1">
									{t('servers.connect.unsupported_detected', {
										os: server.facts.os ?? t('servers.connect.unknown'),
										version: server.facts.version ?? '',
									})}
								</p>
							</div>
						{:else if server?.host_key_fingerprint}
							<div class="space-y-4">
								<div
									class="flex items-start gap-3 rounded-lg border border-success/40 bg-success/5 p-3">
									<CheckIcon class="mt-0.5 size-4 shrink-0 text-success" />
									<div class="min-w-0">
										<p class="text-sm font-medium">{t('servers.connect.connected_pinned')}</p>
										<p class="font-mono text-xs break-all text-muted-foreground">
											{server.host_key_fingerprint}
										</p>
									</div>
								</div>
								<dl class="grid grid-cols-2 gap-x-4 gap-y-2 text-sm sm:grid-cols-3">
									{#each [[t('servers.connect.fact_os'), [server.facts.os, server.facts.version]
												.filter(Boolean)
												.join(' ') || '—'], [t('servers.connect.fact_architecture'), server.facts.architecture ?? '—'], [t('servers.connect.fact_podman'), server.facts.podman_version ?? t('servers.connect.not_installed')], [t('servers.connect.fact_cpu_cores'), server.facts.capacity.cpu_cores?.toString() ?? '—'], [t('servers.connect.fact_memory'), server.facts.capacity.memory_bytes ? `${(server.facts.capacity.memory_bytes / 1024 ** 3).toFixed(1)} GiB` : '—'], [t('servers.connect.fact_free_disk'), server.facts.capacity.disk_free_bytes ? `${(server.facts.capacity.disk_free_bytes / 1024 ** 3).toFixed(1)} GiB` : '—']] as [label, value] (label)}
										<div>
											<dt class="text-xs text-muted-foreground">{label}</dt>
											<dd class="truncate">{value}</dd>
										</div>
									{/each}
								</dl>
								<p class="text-sm text-muted-foreground">
									{t('servers.connect.next_install', {
										mode: podmanMode,
										caddy: ingress === 'public_edge' ? t('servers.connect.next_caddy') : '',
									})}
								</p>
							</div>
						{:else}
							<div class="flex items-center gap-2 text-sm text-muted-foreground">
								<LoaderCircleIcon class="size-4 animate-spin" />
								{t('servers.connect.contacting')}
							</div>
						{/if}
					</div>
				{:else}
					<div class="space-y-4">
						<div>
							<h3 class="text-sm font-medium">
								{t('servers.connect.installing', { mode: podmanMode })}
							</h3>
							<p class="mt-1 text-sm text-muted-foreground">
								{t('servers.connect.install_hint')}
							</p>
						</div>
						<ServerJobLog
							{organizationId}
							serverId={serverId ?? ''}
							onFinished={(finishedJob, latest) => {
								job = finishedJob;
								server = latest;
							}} />
						{#if server?.state === 'ready' && server.facts.swap_recommended}
							<ServerSwapNotice
								{organizationId}
								{server}
								onCreated={async () => {
									if (server) server = await healthcheckServer(organizationId, server.id);
								}} />
						{/if}
					</div>
				{/if}
			</div>
		</div>

		<Dialog.Footer class="shrink-0 border-t px-6 py-4">
			{#if step === 0}
				<Button variant="outline" onclick={() => (open = false)}
					>{t('organizations.common.cancel')}</Button>
				<Button onclick={continueToKey}>{t('servers.connect.continue')}</Button>
			{:else if step === 1}
				<!-- Going back is only safe before the server exists; later steps stay one-way. -->
				<Button
					variant="outline"
					onclick={() => {
						stepError = '';
						step = 0;
					}}>
					{t('servers.connect.back')}
				</Button>
				<Button onclick={() => void register()} disabled={busy}>
					{#if busy}<LoaderCircleIcon class="size-4 animate-spin" />{/if}
					{busy ? t('servers.connect.registering') : t('servers.connect.register')}
				</Button>
			{:else if step === 2}
				<Button variant="outline" onclick={close}>{t('servers.connect.finish_later')}</Button>
				<Button onclick={() => void testConnection()} disabled={busy || !publicKey}>
					{#if busy}<LoaderCircleIcon class="size-4 animate-spin" />{/if}
					{busy ? t('servers.connect.connecting') : t('servers.connect.test_connection')}
				</Button>
			{:else if step === 3}
				<Button variant="outline" onclick={close}>{t('servers.connect.finish_later')}</Button>
				{#if fingerprint}
					<Button onclick={() => void trustFingerprint()} disabled={busy}>
						{#if busy}<LoaderCircleIcon class="size-4 animate-spin" />{/if}
						{busy ? t('servers.connect.confirming') : t('servers.connect.trust_fingerprint')}
					</Button>
				{:else if server?.state === 'unsupported'}
					<Button onclick={close}>{t('organizations.common.close')}</Button>
				{:else if server?.host_key_fingerprint}
					<Button onclick={() => void install()} disabled={busy}>
						{#if busy}<LoaderCircleIcon class="size-4 animate-spin" />{/if}
						{busy ? t('servers.connect.starting') : t('servers.connect.install_configure')}
					</Button>
				{:else}
					<Button onclick={() => void testConnection()} disabled={busy}
						>{t('servers.connect.retry_connection')}</Button>
				{/if}
			{:else}
				<Button variant="outline" onclick={close}
					>{finished
						? t('organizations.common.close')
						: t('servers.connect.run_background')}</Button>
				{#if job?.status === 'failed'}
					<Button onclick={() => void install(true)} disabled={busy}
						>{t('servers.connect.retry_repair')}</Button>
				{:else if finished}
					<Button
						onclick={() => {
							toast.success(
								t('servers.connect.ready_toast', {
									name: server?.name ?? t('organizations.common.server'),
								}),
							);
							close();
						}}>
						{t('organizations.common.done')}
					</Button>
				{/if}
			{/if}
		</Dialog.Footer>
	</Dialog.Content>
</Dialog.Root>
