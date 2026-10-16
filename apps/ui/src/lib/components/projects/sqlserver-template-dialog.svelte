<script lang="ts">
	import * as Dialog from '#lib/components/ui/dialog/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import SoftCheckbox from '#lib/components/custom/soft-checkbox.svelte';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Label } from '#lib/components/ui/label/index.js';
	import * as Select from '#lib/components/ui/select/index.js';
	import CopyButton from '#lib/components/custom/copy-button.svelte';
	import {
		createSqlServerTemplate,
		previewSqlServerTemplate,
		type SqlServerTemplateCreated,
		type SqlServerTemplateInput,
		type SqlServerTemplatePreview,
		type ServerSummaryDto,
	} from '#lib/clients/project-client.js';
	import { maskConnectionStringPassword } from '#lib/utils/connection-string.js';
	import { toast } from '#lib/utils/toast.js';
	import { t } from '#lib/i18n/index.svelte.js';
	import { suggestedResourceName } from '#lib/utils/suggested-resource-name.js';

	const DEFAULT_IMAGE = 'mcr.microsoft.com/mssql/server:2025-latest';
	const MIN_MEMORY_MIB = 2048;

	let {
		organizationId,
		projectId,
		environmentId,
		servers = [],
		serverId,
		onCreated,
		onBack,
		onCancel,
	}: {
		organizationId: string;
		projectId: string;
		environmentId: string;
		servers?: ServerSummaryDto[];
		serverId: string;
		onCreated: (created: SqlServerTemplateCreated) => Promise<void>;
		onBack: () => void;
		onCancel: () => void;
	} = $props();
	const serverTriggerLabel = $derived.by(() => {
		const match = servers.find((server) => server.id === serverId);
		return match
			? `${match.name} · ${match.ingress_mode.replace('_', ' ')}`
			: t('projects.template_common.choose_ready_server');
	});
	let step = $state<'configure' | 'review'>('configure');
	let applicationName = $state(suggestedResourceName('sqlserver'));
	let description = $state('');
	let dockerImage = $state(DEFAULT_IMAGE);
	let password = $state('');
	let volumeName = $state('');
	let publicAccess = $state(false);
	let hostPort = $state<number | ''>('');
	let runtimeCpuCores = $state<number | ''>('');
	let runtimeMemoryMib = $state<number | ''>('');
	let preview = $state<SqlServerTemplatePreview | null>(null);
	let previewing = $state(false);
	let creating = $state(false);
	// SQL Server refuses to start unless the sa password satisfies its own complexity policy.
	function randomFrom(alphabet: string, count: number) {
		const bytes = crypto.getRandomValues(new Uint8Array(count));
		return Array.from(bytes, (byte) => alphabet[byte % alphabet.length]);
	}
	function randomPassword() {
		const upper = 'ABCDEFGHJKLMNPQRSTUVWXYZ';
		const lower = 'abcdefghijkmnopqrstuvwxyz';
		const digits = '23456789';
		const symbols = '!*+,-.:<>^_~';
		const characters = [
			...randomFrom(upper, 1),
			...randomFrom(lower, 1),
			...randomFrom(digits, 1),
			...randomFrom(symbols, 1),
			...randomFrom(upper + lower + digits + symbols, 20),
		];
		for (let index = characters.length - 1; index > 0; index -= 1) {
			const target = crypto.getRandomValues(new Uint32Array(1))[0] % (index + 1);
			[characters[index], characters[target]] = [characters[target], characters[index]];
		}
		return characters.join('');
	}
	function passwordPolicyError(value: string) {
		if (value !== value.trim() || /[\n\r]/.test(value))
			return t('projects.template_sqlserver.password_whitespace');
		if (/['"`\\$]/.test(value)) return t('projects.template_sqlserver.password_quotes');
		if (/[;=]/.test(value)) return t('projects.template_sqlserver.password_semicolons');
		if (value.length < 8 || value.length > 128)
			return t('projects.template_sqlserver.password_length');
		const classes = [/[A-Z]/, /[a-z]/, /[0-9]/, /[^A-Za-z0-9]/].filter((pattern) =>
			pattern.test(value),
		).length;
		if (classes < 3) return t('projects.template_sqlserver.password_complexity');
		return null;
	}
	function reset() {
		step = 'configure';
		applicationName = suggestedResourceName('sqlserver');
		description = '';
		dockerImage = DEFAULT_IMAGE;
		password = randomPassword();
		volumeName = '';
		publicAccess = false;
		hostPort = '';
		runtimeCpuCores = '';
		runtimeMemoryMib = '';
		preview = null;
	}
	reset();
	function optionalNumber(value: number | '') {
		return value === '' || !Number.isFinite(value) ? undefined : value;
	}
	function input(): SqlServerTemplateInput {
		return {
			application_name: applicationName.trim(),
			description: description.trim() || undefined,
			server_id: serverId,
			docker_image: dockerImage.trim() || undefined,
			password: password || undefined,
			volume_name: volumeName.trim() || undefined,
			public_host_port: publicAccess ? optionalNumber(hostPort) : null,
			runtime_cpu_cores: optionalNumber(runtimeCpuCores) ?? null,
			runtime_memory_mib: optionalNumber(runtimeMemoryMib) ?? null,
			template_instance_id: preview?.template_instance_id,
		};
	}
	function validateConfiguration() {
		if (!applicationName.trim()) {
			toast.error(t('projects.template_common.application_name_required'));
			return false;
		}
		const policyError = passwordPolicyError(password);
		if (policyError) {
			toast.error(policyError);
			return false;
		}
		const port = optionalNumber(hostPort);
		if (
			publicAccess &&
			(port === undefined || !Number.isInteger(port) || port < 1 || port > 65535)
		) {
			toast.error(t('projects.template_common.public_port_invalid'));
			return false;
		}
		if (
			(runtimeCpuCores !== '' || runtimeMemoryMib !== '') &&
			(!optionalNumber(runtimeCpuCores) || !optionalNumber(runtimeMemoryMib))
		) {
			toast.error(t('projects.template_common.runtime_limits_both_required'));
			return false;
		}
		const memory = optionalNumber(runtimeMemoryMib);
		if (memory !== undefined && memory < MIN_MEMORY_MIB) {
			toast.error(t('projects.template_sqlserver.memory_required', { count: MIN_MEMORY_MIB }));
			return false;
		}
		return true;
	}
	async function review() {
		if (!validateConfiguration()) return;
		previewing = true;
		try {
			preview = await previewSqlServerTemplate(organizationId, projectId, environmentId, input());
			password = preview.resolved_password;
			step = 'review';
		} catch (error) {
			toast.error(
				error instanceof Error ? error.message : t('projects.template_sqlserver.preview_failed'),
			);
		} finally {
			previewing = false;
		}
	}
	async function create() {
		if (!preview) return;
		creating = true;
		try {
			const created = await createSqlServerTemplate(
				organizationId,
				projectId,
				environmentId,
				input(),
			);
			await onCreated(created);
			onCancel();
		} catch (error) {
			toast.error(
				error instanceof Error ? error.message : t('projects.template_sqlserver.create_failed'),
			);
		} finally {
			creating = false;
		}
	}
</script>

<Dialog.Header class="shrink-0"
	><Dialog.Title
		>{step === 'configure'
			? t('projects.template_sqlserver.create_title')
			: t('projects.template_sqlserver.review_title')}</Dialog.Title
	><Dialog.Description
		>{step === 'configure'
			? t('projects.template_sqlserver.create_description')
			: t('projects.template_sqlserver.review_description')}</Dialog.Description
	></Dialog.Header>
{#if step === 'configure'}<form
		class="flex min-h-0 flex-1 flex-col overflow-hidden"
		onsubmit={(event) => {
			event.preventDefault();
			void review();
		}}>
		<div class="grid min-h-0 flex-1 gap-5 overflow-y-auto pr-1">
			<div class="grid gap-4 sm:grid-cols-2">
				<div class="grid gap-2">
					<Label for="sqlserver-name">{t('projects.projects.name')}</Label><Input
						id="sqlserver-name"
						bind:value={applicationName}
						placeholder="sqlserver-main"
						autofocus
						onfocus={(event) => event.currentTarget.select()} />
				</div>
				<div class="grid gap-2">
					<Label for="sqlserver-image">{t('projects.template_common.docker_image')}</Label><Input
						id="sqlserver-image"
						bind:value={dockerImage} />
					<p class="text-sm text-muted-foreground">
						{t('projects.template_sqlserver.image_hint', { count: MIN_MEMORY_MIB })}
					</p>
				</div>
				<div class="grid gap-2 sm:col-span-2">
					<Label for="sqlserver-server">{t('organizations.common.server')}</Label><Select.Root
						type="single"
						bind:value={serverId}>
						<Select.Trigger id="sqlserver-server" class="w-full"
							>{serverTriggerLabel}</Select.Trigger>
						<Select.Content>
							{#each servers as server (server.id)}
								<Select.Item value={server.id}
									>{server.name} · {server.ingress_mode.replace('_', ' ')}</Select.Item>
							{/each}
						</Select.Content>
					</Select.Root>{#if servers.length === 0}<p class="text-sm text-destructive">
							{t('projects.template_common.no_ready_servers')}
						</p>{/if}
				</div>
				<div class="grid gap-2 sm:col-span-2">
					<Label for="sqlserver-description"
						>{t('projects.environment.description')}
						<span class="text-muted-foreground">({t('projects.environment.optional')})</span></Label
					><Input id="sqlserver-description" bind:value={description} />
				</div>
				<div class="grid gap-2 sm:col-span-2">
					<div class="flex items-center justify-between">
						<Label for="sqlserver-password">{t('projects.template_sqlserver.sa_password')}</Label
						><Button
							type="button"
							variant="ghost"
							size="sm"
							onclick={() => (password = randomPassword())}>{t('account.common.generate')}</Button>
					</div>
					<Input
						id="sqlserver-password"
						type="password"
						bind:value={password}
						autocomplete="new-password" />
					<p class="text-sm text-muted-foreground">
						{t('projects.template_sqlserver.password_hint')}
					</p>
				</div>
				<div class="grid gap-2 sm:col-span-2">
					<Label for="sqlserver-volume"
						>{t('projects.template_common.persistent_volume_name')}
						<span class="text-muted-foreground">({t('projects.environment.optional')})</span></Label
					><Input id="sqlserver-volume" bind:value={volumeName} placeholder="sqlserver-main-data" />
				</div>
			</div>
			<div class="rounded-lg border p-4">
				<div class="flex items-start gap-3">
					<SoftCheckbox id="sqlserver-public" bind:checked={publicAccess} class="mt-0.5" />
					<div class="grid gap-1">
						<Label for="sqlserver-public"
							>{t('projects.template_sqlserver.publish_publicly')}</Label>
						<p class="text-sm text-muted-foreground">
							{t('projects.template_sqlserver.publish_instance_hint')}
						</p>
					</div>
				</div>
				{#if publicAccess}<div class="mt-4 grid gap-2 sm:max-w-xs">
						<Label for="sqlserver-port">{t('projects.template_common.public_host_port')}</Label
						><Input
							id="sqlserver-port"
							type="number"
							min="1"
							max="65535"
							bind:value={hostPort}
							placeholder="1433" />
					</div>{/if}
			</div>
			<details class="rounded-lg border p-4">
				<summary class="cursor-pointer font-medium"
					>{t('projects.template_common.runtime_resource_limits')}
					<span class="font-normal text-muted-foreground"
						>({t('projects.environment.optional')})</span
					></summary>
				<div class="mt-4 grid gap-4 sm:grid-cols-2">
					<div class="grid gap-2">
						<Label for="sqlserver-cpu">{t('projects.template_common.cpu_cores')}</Label><Input
							id="sqlserver-cpu"
							type="number"
							min="0.1"
							step="0.1"
							bind:value={runtimeCpuCores} />
					</div>
					<div class="grid gap-2">
						<Label for="sqlserver-memory">{t('projects.template_common.memory_mib')}</Label><Input
							id="sqlserver-memory"
							type="number"
							min={MIN_MEMORY_MIB}
							step="1"
							bind:value={runtimeMemoryMib}
							placeholder="4096" />
					</div>
				</div>
			</details>
		</div>
		<Dialog.Footer class="shrink-0 border-t border-border bg-popover pt-4"
			><Button type="button" variant="outline" onclick={onBack}
				>{t('projects.template_common.back')}</Button
			><Button type="button" variant="outline" onclick={onCancel}
				>{t('projects.projects.cancel')}</Button
			><Button type="submit" disabled={previewing}
				>{previewing
					? t('projects.template_common.preparing')
					: t('projects.template_common.review_configuration')}</Button
			></Dialog.Footer>
	</form>
{:else if preview}<div class="flex min-h-0 flex-1 flex-col overflow-hidden">
		<div class="grid min-h-0 flex-1 gap-5 overflow-y-auto pr-1">
			<div class="grid gap-3 rounded-lg border bg-muted/20 p-4 text-sm">
				<div class="flex flex-wrap items-center justify-between gap-2">
					<span class="text-muted-foreground">{t('projects.template_common.private_alias')}</span
					><code class="rounded bg-background px-2 py-1">{preview.private_alias}:1433</code>
				</div>
				<div class="flex items-center justify-between gap-3">
					<span class="text-muted-foreground"
						>{t('projects.template_sqlserver.private_connection_url')}</span
					><CopyButton
						value={preview.private_connection_string}
						label={maskConnectionStringPassword(preview.private_connection_string)}
						class="max-w-[70%] font-mono text-xs"
						errorMessage={t('projects.template_common.clipboard_unavailable')} />
				</div>
				<div class="flex items-center justify-between gap-3">
					<span class="text-muted-foreground">{t('projects.template_sqlserver.private_ado')}</span
					><CopyButton
						value={preview.private_ado_connection_string}
						label={maskConnectionStringPassword(preview.private_ado_connection_string)}
						class="max-w-[70%] font-mono text-xs"
						errorMessage={t('projects.template_common.clipboard_unavailable')} />
				</div>
				{#if preview.public_connection_string}<div class="flex items-center justify-between gap-3">
						<span class="text-muted-foreground"
							>{t('projects.template_sqlserver.public_connection_url')}</span
						><CopyButton
							value={preview.public_connection_string}
							label={maskConnectionStringPassword(preview.public_connection_string)}
							class="max-w-[70%] font-mono text-xs"
							errorMessage={t('projects.template_common.clipboard_unavailable')} />
					</div>
					{#if preview.public_ado_connection_string}<div
							class="flex items-center justify-between gap-3">
							<span class="text-muted-foreground"
								>{t('projects.template_sqlserver.public_ado')}</span
							><CopyButton
								value={preview.public_ado_connection_string}
								label={maskConnectionStringPassword(preview.public_ado_connection_string)}
								class="max-w-[70%] font-mono text-xs"
								errorMessage={t('projects.template_common.clipboard_unavailable')} />
						</div>{/if}
					<p class="rounded bg-warning/10 p-3 text-warning">
						{t('projects.template_sqlserver.public_warning')}
					</p>{/if}
			</div>
			<div class="grid gap-2">
				<p class="text-sm font-medium">{t('projects.application_general.docker_compose')}</p>
				<pre class="overflow-x-auto rounded-lg border bg-muted/30 p-4 text-xs"><code
						>{preview.compose}</code></pre>
			</div>
			<div class="grid gap-3 sm:grid-cols-2">
				<div class="rounded-lg border p-4">
					<p class="text-sm font-medium">{t('projects.template_common.runtime_variable')}</p>
					<ul class="mt-2 grid gap-1 font-mono text-xs">
						{#each preview.variables as variable}<li>
								{variable.name}{variable.masked ? '=••••••••' : ''}
							</li>{/each}
					</ul>
				</div>
				<div class="rounded-lg border p-4">
					<p class="text-sm font-medium">{t('projects.template_common.volume')}</p>
					<ul class="mt-2 grid gap-1 text-sm">
						{#each preview.volumes as volume}<li>
								<code>{volume.name}</code> → <code>{volume.mount_path}</code>
							</li>{/each}
					</ul>
					{#if preview.port_mappings.length}<p class="mt-3 text-sm text-muted-foreground">
							{preview.port_mappings[0].host_port} → {preview.port_mappings[0].container_port}
						</p>{/if}
				</div>
			</div>
		</div>
		<Dialog.Footer class="shrink-0 border-t border-border bg-popover pt-4"
			><Button variant="outline" onclick={() => (step = 'configure')}
				>{t('projects.template_common.back')}</Button
			><Button onclick={() => void create()} disabled={creating}
				>{creating
					? t('projects.volume.creating')
					: t('projects.template_sqlserver.create_button')}</Button
			></Dialog.Footer>
	</div>{/if}
