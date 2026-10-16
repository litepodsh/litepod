<script lang="ts">
	import * as Dialog from '#lib/components/ui/dialog/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import SoftCheckbox from '#lib/components/custom/soft-checkbox.svelte';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Label } from '#lib/components/ui/label/index.js';
	import * as Select from '#lib/components/ui/select/index.js';
	import CopyButton from '#lib/components/custom/copy-button.svelte';
	import {
		createDragonflyDbTemplate,
		previewDragonflyDbTemplate,
		type DragonflyDbTemplateCreated,
		type DragonflyDbTemplateInput,
		type DragonflyDbTemplatePreview,
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
		onCreated: (created: DragonflyDbTemplateCreated) => Promise<void>;
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
	let applicationName = $state(suggestedResourceName('dragonflydb'));
	let description = $state('');
	let dockerImage = $state('docker.dragonflydb.io/dragonflydb/dragonfly');
	let password = $state('');
	let volumeName = $state('');
	let snapshotCron = $state('*/5 * * * *');
	let maxMemoryMib = $state<number | ''>('');
	let cacheMode = $state(false);
	let publicAccess = $state(false);
	let hostPort = $state<number | ''>('');
	let runtimeCpuCores = $state<number | ''>('');
	let runtimeMemoryMib = $state<number | ''>('');
	let preview = $state<DragonflyDbTemplatePreview | null>(null);
	let previewing = $state(false);
	let creating = $state(false);
	function randomPassword() {
		const alphabet = 'ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789-_=!';
		const bytes = crypto.getRandomValues(new Uint8Array(24));
		return Array.from(bytes, (byte) => alphabet[byte % alphabet.length]).join('');
	}
	function reset() {
		step = 'configure';
		applicationName = suggestedResourceName('dragonflydb');
		description = '';
		dockerImage = 'docker.dragonflydb.io/dragonflydb/dragonfly';
		password = randomPassword();
		volumeName = '';
		snapshotCron = '*/5 * * * *';
		maxMemoryMib = '';
		cacheMode = false;
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
	function input(): DragonflyDbTemplateInput {
		return {
			application_name: applicationName.trim(),
			description: description.trim() || undefined,
			server_id: serverId,
			docker_image: dockerImage.trim() || undefined,
			password: password || undefined,
			snapshot_cron: snapshotCron.trim() || undefined,
			max_memory_mib: optionalNumber(maxMemoryMib) ?? null,
			cache_mode: cacheMode,
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
		const port = optionalNumber(hostPort);
		if (
			publicAccess &&
			(port === undefined || !Number.isInteger(port) || port < 1 || port > 65535)
		) {
			toast.error(t('projects.template_common.public_port_invalid'));
			return false;
		}
		const memory = optionalNumber(maxMemoryMib);
		if (cacheMode && (memory === undefined || !Number.isInteger(memory) || memory < 1)) {
			toast.error(t('projects.template_dragonfly.max_memory_cache_invalid'));
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
			preview = await previewDragonflyDbTemplate(organizationId, projectId, environmentId, input());
			password = preview.resolved_password;
			step = 'review';
		} catch (error) {
			toast.error(
				error instanceof Error ? error.message : t('projects.template_dragonfly.preview_failed'),
			);
		} finally {
			previewing = false;
		}
	}
	async function create() {
		if (!preview) return;
		creating = true;
		try {
			const created = await createDragonflyDbTemplate(
				organizationId,
				projectId,
				environmentId,
				input(),
			);
			await onCreated(created);
			onCancel();
		} catch (error) {
			toast.error(
				error instanceof Error ? error.message : t('projects.template_dragonfly.create_failed'),
			);
		} finally {
			creating = false;
		}
	}
</script>

<Dialog.Header class="shrink-0"
	><Dialog.Title
		>{step === 'configure'
			? t('projects.template_dragonfly.create_title')
			: t('projects.template_dragonfly.review_title')}</Dialog.Title
	><Dialog.Description
		>{step === 'configure'
			? t('projects.template_common.create_service_description', { name: 'DragonflyDB' })
			: t('projects.template_common.redis_review_description')}</Dialog.Description
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
					<Label for="dragonfly-name">{t('projects.projects.name')}</Label><Input
						id="dragonfly-name"
						bind:value={applicationName}
						placeholder="dragonfly-main"
						autofocus
						onfocus={(event) => event.currentTarget.select()} />
				</div>
				<div class="grid gap-2">
					<Label for="dragonfly-image">{t('projects.template_common.docker_image')}</Label><Input
						id="dragonfly-image"
						bind:value={dockerImage} />
				</div>
				<div class="grid gap-2 sm:col-span-2">
					<Label for="dragonfly-server">{t('organizations.common.server')}</Label><Select.Root
						type="single"
						bind:value={serverId}>
						<Select.Trigger id="dragonfly-server" class="w-full"
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
					<Label for="dragonfly-description"
						>{t('projects.environment.description')}
						<span class="text-muted-foreground">({t('projects.environment.optional')})</span></Label
					><Input id="dragonfly-description" bind:value={description} />
				</div>
				<div class="grid gap-2 sm:col-span-2">
					<div class="flex items-center justify-between">
						<Label for="dragonfly-password">{t('auth.login.password')}</Label><Button
							type="button"
							variant="ghost"
							size="sm"
							onclick={() => (password = randomPassword())}>{t('account.common.generate')}</Button>
					</div>
					<Input
						id="dragonfly-password"
						type="password"
						bind:value={password}
						autocomplete="new-password" />
				</div>
				<div class="grid gap-2">
					<Label for="dragonfly-volume"
						>{t('projects.template_common.persistent_volume_name')}
						<span class="text-muted-foreground">({t('projects.environment.optional')})</span></Label
					><Input id="dragonfly-volume" bind:value={volumeName} placeholder="dragonfly-main-data" />
				</div>
				<div class="grid gap-2">
					<Label for="dragonfly-cron">{t('projects.template_dragonfly.snapshot_cron')}</Label><Input
						id="dragonfly-cron"
						bind:value={snapshotCron} />
				</div>
				<div class="grid gap-2">
					<Label for="dragonfly-max-memory">{t('projects.template_common.max_memory')}</Label><Input
						id="dragonfly-max-memory"
						type="number"
						min="1"
						step="1"
						bind:value={maxMemoryMib}
						placeholder="512" />
				</div>
				<div class="rounded-lg border p-3">
					<div class="flex items-start gap-3">
						<SoftCheckbox id="dragonfly-cache-mode" bind:checked={cacheMode} class="mt-0.5" />
						<div class="grid gap-1">
							<Label for="dragonfly-cache-mode"
								>{t('projects.template_dragonfly.cache_mode')}</Label>
							<p class="text-sm text-muted-foreground">
								{t('projects.template_dragonfly.cache_mode_hint')}
							</p>
						</div>
					</div>
				</div>
			</div>
			<div class="rounded-lg border p-4">
				<div class="flex items-start gap-3">
					<SoftCheckbox id="dragonfly-public" bind:checked={publicAccess} class="mt-0.5" />
					<div class="grid gap-1">
						<Label for="dragonfly-public"
							>{t('projects.template_dragonfly.publish_publicly')}</Label>
						<p class="text-sm text-muted-foreground">
							{t('projects.template_common.publish_service_hint')}
						</p>
					</div>
				</div>
				{#if publicAccess}<div class="mt-4 grid gap-2 sm:max-w-xs">
						<Label for="dragonfly-port">{t('projects.template_common.public_host_port')}</Label
						><Input
							id="dragonfly-port"
							type="number"
							min="1"
							max="65535"
							bind:value={hostPort}
							placeholder="26379" />
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
						<Label for="dragonfly-cpu">{t('projects.template_common.cpu_cores')}</Label><Input
							id="dragonfly-cpu"
							type="number"
							min="0.1"
							step="0.1"
							bind:value={runtimeCpuCores} />
					</div>
					<div class="grid gap-2">
						<Label for="dragonfly-memory">{t('projects.template_common.memory_mib')}</Label><Input
							id="dragonfly-memory"
							type="number"
							min="1"
							step="1"
							bind:value={runtimeMemoryMib} />
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
					><code class="rounded bg-background px-2 py-1">{preview.private_alias}:6379</code>
				</div>
				<div class="flex items-center justify-between gap-3">
					<span class="text-muted-foreground"
						>{t('projects.template_common.private_redis_url')}</span
					><CopyButton
						value={preview.private_connection_string}
						label={maskConnectionStringPassword(preview.private_connection_string)}
						class="max-w-[70%] font-mono text-xs"
						errorMessage={t('projects.template_common.clipboard_unavailable')} />
				</div>
				{#if preview.public_connection_string}<div class="flex items-center justify-between gap-3">
						<span class="text-muted-foreground"
							>{t('projects.template_common.public_redis_url')}</span
						><CopyButton
							value={preview.public_connection_string}
							label={maskConnectionStringPassword(preview.public_connection_string)}
							class="max-w-[70%] font-mono text-xs"
							errorMessage={t('projects.template_common.clipboard_unavailable')} />
					</div>
					<p class="rounded bg-warning/10 p-3 text-warning">
						{t('projects.template_common.redis_tcp_warning')}
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
					: t('projects.template_dragonfly.create_title')}</Button
			></Dialog.Footer>
	</div>{/if}
