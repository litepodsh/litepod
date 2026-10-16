<script lang="ts">
	import * as Dialog from '#lib/components/ui/dialog/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import SoftCheckbox from '#lib/components/custom/soft-checkbox.svelte';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Label } from '#lib/components/ui/label/index.js';
	import * as Select from '#lib/components/ui/select/index.js';
	import CopyButton from '#lib/components/custom/copy-button.svelte';
	import {
		createPostgresTemplate,
		previewPostgresTemplate,
		type PostgresTemplateCreated,
		type PostgresTemplateInput,
		type PostgresTemplatePreview,
		type ServerSummaryDto,
	} from '#lib/clients/project-client.js';
	import { maskConnectionStringPassword } from '#lib/utils/connection-string.js';
	import { toast } from '#lib/utils/toast.js';
	import { t } from '#lib/i18n/index.svelte.js';
	import { suggestedResourceName } from '#lib/utils/suggested-resource-name.js';

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
		onCreated: (created: PostgresTemplateCreated) => Promise<void>;
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
	let applicationName = $state(suggestedResourceName('postgres'));
	let description = $state('');
	let dockerImage = $state('postgres:17.2');
	let databaseName = $state('postgres');
	let databaseUser = $state('postgres');
	let password = $state('');
	let volumeName = $state('');
	let publicAccess = $state(false);
	let hostPort = $state<number | ''>('');
	let runtimeCpuCores = $state<number | ''>('');
	let runtimeMemoryMib = $state<number | ''>('');
	let preview = $state<PostgresTemplatePreview | null>(null);
	let previewing = $state(false);
	let creating = $state(false);

	function reset() {
		step = 'configure';
		applicationName = suggestedResourceName('postgres');
		description = '';
		dockerImage = 'postgres:17.2';
		databaseName = 'postgres';
		databaseUser = 'postgres';
		password = randomPassword();
		volumeName = '';
		publicAccess = false;
		hostPort = '';
		runtimeCpuCores = '';
		runtimeMemoryMib = '';
		preview = null;
	}

	reset();

	function randomPassword(): string {
		const alphabet = 'ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789-_=!';
		const bytes = crypto.getRandomValues(new Uint8Array(24));
		return Array.from(bytes, (byte) => alphabet[byte % alphabet.length]).join('');
	}

	function generatePassword() {
		password = randomPassword();
	}

	function optionalNumber(value: number | ''): number | undefined {
		return value === '' || !Number.isFinite(value) ? undefined : value;
	}

	function input(): PostgresTemplateInput {
		return {
			application_name: applicationName.trim(),
			description: description.trim() || undefined,
			server_id: serverId,
			docker_image: dockerImage.trim() || undefined,
			database_name: databaseName.trim() || undefined,
			database_user: databaseUser.trim() || undefined,
			password: password || undefined,
			volume_name: volumeName.trim() || undefined,
			public_host_port: publicAccess ? optionalNumber(hostPort) : null,
			runtime_cpu_cores: optionalNumber(runtimeCpuCores) ?? null,
			runtime_memory_mib: optionalNumber(runtimeMemoryMib) ?? null,
			template_instance_id: preview?.template_instance_id,
		};
	}

	function validateConfiguration(): boolean {
		if (!applicationName.trim()) {
			toast.error(t('projects.template_common.application_name_required'));
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
		return true;
	}

	async function review() {
		if (!validateConfiguration()) return;
		previewing = true;
		try {
			preview = await previewPostgresTemplate(organizationId, projectId, environmentId, input());
			password = preview.resolved_password;
			step = 'review';
		} catch (error) {
			toast.error(
				error instanceof Error ? error.message : t('projects.template_postgres.preview_failed'),
			);
		} finally {
			previewing = false;
		}
	}

	async function create() {
		if (!preview) return;
		creating = true;
		try {
			const created = await createPostgresTemplate(
				organizationId,
				projectId,
				environmentId,
				input(),
			);
			await onCreated(created);
			onCancel();
		} catch (error) {
			toast.error(
				error instanceof Error ? error.message : t('projects.template_postgres.create_failed'),
			);
		} finally {
			creating = false;
		}
	}
</script>

<Dialog.Header class="shrink-0">
	<Dialog.Title
		>{step === 'configure'
			? t('projects.template_postgres.create_title')
			: t('projects.template_postgres.review_title')}</Dialog.Title>
	<Dialog.Description>
		{step === 'configure'
			? t('projects.template_common.create_db_description', { name: 'PostgreSQL' })
			: t('projects.template_common.review_description')}
	</Dialog.Description>
</Dialog.Header>

{#if step === 'configure'}
	<form
		class="flex min-h-0 flex-1 flex-col overflow-hidden"
		onsubmit={(event) => {
			event.preventDefault();
			void review();
		}}>
		<div class="grid min-h-0 flex-1 gap-5 overflow-y-auto pr-1">
			<div class="grid gap-4 sm:grid-cols-2">
				<div class="grid gap-2">
					<Label for="postgres-application-name">{t('projects.projects.name')}</Label>
					<Input
						id="postgres-application-name"
						bind:value={applicationName}
						placeholder="postgres-main"
						autofocus
						onfocus={(event) => event.currentTarget.select()} />
				</div>
				<div class="grid gap-2">
					<Label for="postgres-image">{t('projects.template_common.docker_image')}</Label>
					<Input
						id="postgres-image"
						bind:value={dockerImage}
						placeholder="supabase/postgres:17.2.3" />
				</div>
				<div class="grid gap-2 sm:col-span-2">
					<Label for="postgres-server">{t('organizations.common.server')}</Label>
					<Select.Root type="single" bind:value={serverId}>
						<Select.Trigger id="postgres-server" class="w-full"
							>{serverTriggerLabel}</Select.Trigger>
						<Select.Content>
							{#each servers as server (server.id)}
								<Select.Item value={server.id}
									>{server.name} · {server.ingress_mode.replace('_', ' ')}</Select.Item>
							{/each}
						</Select.Content>
					</Select.Root>
					{#if servers.length === 0}<p class="text-sm text-destructive">
							{t('projects.template_common.no_ready_servers')}
						</p>{/if}
				</div>
				<div class="grid gap-2">
					<Label for="postgres-database">{t('projects.template_common.database_name')}</Label>
					<Input id="postgres-database" bind:value={databaseName} />
				</div>
				<div class="grid gap-2">
					<Label for="postgres-user">{t('projects.template_common.database_user')}</Label>
					<Input id="postgres-user" bind:value={databaseUser} />
				</div>
				<div class="grid gap-2 sm:col-span-2">
					<div class="flex items-center justify-between gap-3">
						<Label for="postgres-password">{t('auth.login.password')}</Label>
						<Button type="button" variant="ghost" size="sm" onclick={generatePassword}
							>{t('account.common.generate')}</Button>
					</div>
					<Input
						id="postgres-password"
						type="password"
						bind:value={password}
						placeholder={t('projects.template_postgres.password_placeholder')}
						autocomplete="new-password" />
				</div>
				<div class="grid gap-2 sm:col-span-2">
					<Label for="postgres-volume"
						>{t('projects.template_common.persistent_volume_name')}
						<span class="text-muted-foreground">({t('projects.environment.optional')})</span
						></Label>
					<Input id="postgres-volume" bind:value={volumeName} placeholder="postgres-main-data" />
				</div>
			</div>

			<div class="rounded-lg border p-4">
				<div class="flex items-start gap-3">
					<SoftCheckbox id="postgres-public-access" bind:checked={publicAccess} class="mt-0.5" />
					<div class="grid gap-1">
						<Label for="postgres-public-access"
							>{t('projects.template_postgres.publish_publicly')}</Label>
						<p class="text-sm text-muted-foreground">
							{t('projects.template_common.publish_db_hint', { name: 'PostgreSQL' })}
						</p>
					</div>
				</div>
				{#if publicAccess}
					<div class="mt-4 grid gap-2 sm:max-w-xs">
						<Label for="postgres-public-port"
							>{t('projects.template_common.public_host_port')}</Label>
						<Input
							id="postgres-public-port"
							type="number"
							min="1"
							max="65535"
							bind:value={hostPort}
							placeholder="25432" />
					</div>
				{/if}
			</div>

			<details class="rounded-lg border p-4">
				<summary class="cursor-pointer font-medium"
					>{t('projects.template_common.runtime_resource_limits')}
					<span class="font-normal text-muted-foreground"
						>({t('projects.environment.optional')})</span
					></summary>
				<div class="mt-4 grid gap-4 sm:grid-cols-2">
					<div class="grid gap-2">
						<Label for="postgres-cpu">{t('projects.template_common.cpu_cores')}</Label><Input
							id="postgres-cpu"
							type="number"
							min="0.1"
							step="0.1"
							bind:value={runtimeCpuCores}
							placeholder="0.5" />
					</div>
					<div class="grid gap-2">
						<Label for="postgres-memory">{t('projects.template_common.memory_mib')}</Label><Input
							id="postgres-memory"
							type="number"
							min="1"
							step="1"
							bind:value={runtimeMemoryMib}
							placeholder="512" />
					</div>
				</div>
			</details>
		</div>

		<Dialog.Footer class="shrink-0 border-t border-border bg-popover pt-4">
			<Button type="button" variant="outline" onclick={onBack}
				>{t('projects.template_common.back')}</Button>
			<Button type="button" variant="outline" onclick={onCancel}
				>{t('projects.projects.cancel')}</Button>
			<Button type="submit" disabled={previewing}
				>{previewing
					? t('projects.template_common.preparing')
					: t('projects.template_common.review_configuration')}</Button>
		</Dialog.Footer>
	</form>
{:else if preview}
	<div class="flex min-h-0 flex-1 flex-col overflow-hidden">
		<div class="grid min-h-0 flex-1 gap-5 overflow-y-auto pr-1">
			<div class="grid gap-3 rounded-lg border bg-muted/20 p-4 text-sm">
				<div class="flex flex-wrap items-center justify-between gap-2">
					<span class="text-muted-foreground">{t('projects.template_common.private_alias')}</span
					><code class="rounded bg-background px-2 py-1">{preview.private_alias}:5432</code>
				</div>
				<div class="flex items-center justify-between gap-3">
					<span class="text-muted-foreground"
						>{t('projects.template_postgres.private_connection')}</span
					><CopyButton
						value={preview.private_connection_string}
						label={maskConnectionStringPassword(preview.private_connection_string)}
						class="max-w-[70%] font-mono text-xs"
						errorMessage={t('projects.template_common.clipboard_unavailable')} />
				</div>
				{#if preview.public_connection_string}
					<div class="flex items-center justify-between gap-3">
						<span class="text-muted-foreground"
							>{t('projects.template_postgres.public_connection')}</span
						><CopyButton
							value={preview.public_connection_string}
							label={maskConnectionStringPassword(preview.public_connection_string)}
							class="max-w-[70%] font-mono text-xs"
							errorMessage={t('projects.template_common.clipboard_unavailable')} />
					</div>
					<p class="rounded bg-warning/10 p-3 text-warning">
						{t('projects.template_common.tcp_warning', { name: 'PostgreSQL' })}
					</p>
				{/if}
			</div>

			<div class="grid gap-2">
				<p class="text-sm font-medium">{t('projects.application_general.docker_compose')}</p>
				<pre class="overflow-x-auto rounded-lg border bg-muted/30 p-4 text-xs"><code
						>{preview.compose}</code></pre>
			</div>
			<div class="grid gap-3 sm:grid-cols-2">
				<div class="rounded-lg border p-4">
					<p class="text-sm font-medium">{t('projects.template_common.runtime_variables')}</p>
					<ul class="mt-2 grid gap-1 font-mono text-xs">
						{#each preview.variables as variable}<li>
								{variable.name}{variable.masked ? '=••••••••' : ''}
							</li>{/each}
					</ul>
				</div>
				<div class="rounded-lg border p-4">
					<p class="text-sm font-medium">{t('projects.application.volumes')}</p>
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
		<Dialog.Footer class="shrink-0 border-t border-border bg-popover pt-4">
			<Button variant="outline" onclick={() => (step = 'configure')}
				>{t('projects.template_common.back')}</Button>
			<Button onclick={() => void create()} disabled={creating}
				>{creating
					? t('projects.volume.creating')
					: t('projects.template_postgres.create_title')}</Button>
		</Dialog.Footer>
	</div>
{/if}
