<script lang="ts">
	import SkeletonRows from '#lib/components/custom/skeleton-rows.svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Label } from '#lib/components/ui/label/index.js';
	import CopyButton from '#lib/components/custom/copy-button.svelte';
	import ApplicationComposeTab from '#lib/components/projects/application-compose-tab.svelte';
	import RedeployConfirmDialog from '#lib/components/projects/redeploy-confirm-dialog.svelte';
	import InternalTemplateRuntimeActions from '#lib/components/projects/internal-template-runtime-actions.svelte';
	import { allowedHere } from '#lib/state/permissions.svelte.js';
	import MongoDbIcon from '#lib/components/icons/mongodb-icon.svelte';
	import EyeIcon from '@lucide/svelte/icons/eye';
	import EyeOffIcon from '@lucide/svelte/icons/eye-off';
	import {
		getMongoDbResource,
		rotateMongoDbPassword,
		updateMongoDbResource,
		type ComposeContent,
		type MongoDbResourceDetail,
		type PortMapping,
		type ValidationResponse,
	} from '#lib/clients/project-client.js';
	import { toast } from '#lib/utils/toast.js';
	import { deployLabel } from '#lib/utils/deploy-label.js';
	import { t } from '#lib/i18n/index.svelte.js';

	type Props = {
		organizationId: string;
		projectId: string;
		environmentId: string;
		applicationId: string;
		onDeploy: () => Promise<void>;
		onStart: () => Promise<void>;
		onStop: () => Promise<void>;
		onRestart: () => Promise<void>;
		runtimeBusy?: boolean;
		deploymentStatus: string;
		composeContent: string;
		composePortMappings: PortMapping[];
		runtimeVariableNames: string[];
		onComposeSave: (content: string, ports: PortMapping[]) => Promise<ComposeContent>;
		onComposeDeploy: () => Promise<void>;
		onComposeValidate: (content: string) => Promise<ValidationResponse>;
		onComposeContentChange: (content: string) => void;
		onCreateRuntimeVariables: (created: { name: string; value: string }[]) => Promise<void>;
		mode?: 'overview' | 'configuration';
	};
	let {
		organizationId,
		projectId,
		environmentId,
		applicationId,
		onDeploy,
		onStart,
		onStop,
		onRestart,
		runtimeBusy = false,
		deploymentStatus,
		composeContent,
		composePortMappings,
		runtimeVariableNames,
		onComposeSave,
		onComposeDeploy,
		onComposeValidate,
		onComposeContentChange,
		onCreateRuntimeVariables,
		mode = 'overview',
	}: Props = $props();
	let detail = $state<MongoDbResourceDetail | null>(null);
	let loading = $state(true);
	let busy = $state(false);
	let showPassword = $state(false);
	let showPrivateUrl = $state(false);
	let showPublicUrl = $state(false);
	let image = $state('');
	let publicPort = $state<number | ''>('');
	let cpu = $state<number | ''>('');
	let memory = $state<number | ''>('');
	let password = $state('');
	let redeployOpen = $state(false);
	let redeployPort = $state<number | null>(null);
	async function refresh() {
		loading = true;
		try {
			detail = await getMongoDbResource(organizationId, projectId, environmentId, applicationId);
			image = detail.docker_image;
			publicPort = detail.public_host_port ?? '';
			cpu = detail.runtime_cpu_cores ?? '';
			memory = detail.runtime_memory_mib ?? '';
			password = detail.password ?? '';
		} catch (error) {
			toast.error(
				error instanceof Error ? error.message : t('projects.resource_mongodb.load_failed'),
			);
		} finally {
			loading = false;
		}
	}
	async function savePassword() {
		if (!detail) return;
		busy = true;
		try {
			detail = await updateMongoDbResource(
				organizationId,
				projectId,
				environmentId,
				applicationId,
				{
					docker_image: detail.docker_image,
					public_host_port: detail.public_host_port,
					runtime_cpu_cores: detail.runtime_cpu_cores,
					runtime_memory_mib: detail.runtime_memory_mib,
					password,
				},
			);
			toast.success(t('projects.resource_mongodb.password_saved'));
		} catch (error) {
			toast.error(
				error instanceof Error ? error.message : t('projects.resource_mongodb.password_failed'),
			);
		} finally {
			busy = false;
		}
	}
	async function savePublicPort() {
		if (!detail) return;
		const port = publicPort === '' ? null : publicPort;
		if (port !== null && (!Number.isInteger(port) || port < 1 || port > 65535))
			return toast.error(t('projects.resource_common.port_range_error'));
		busy = true;
		try {
			detail = await updateMongoDbResource(
				organizationId,
				projectId,
				environmentId,
				applicationId,
				{
					docker_image: detail.docker_image,
					public_host_port: port,
					runtime_cpu_cores: detail.runtime_cpu_cores,
					runtime_memory_mib: detail.runtime_memory_mib,
				},
			);
			publicPort = detail.public_host_port ?? '';
			if (detail.config_stale) {
				redeployPort = port;
				redeployOpen = true;
			} else toast.success(t('projects.resource_common.public_access_saved'));
		} catch (error) {
			toast.error(
				error instanceof Error ? error.message : t('projects.resource_common.public_access_failed'),
			);
		} finally {
			busy = false;
		}
	}
	async function saveConfiguration() {
		if (!image.trim()) return toast.error(t('projects.resource_common.docker_image_required'));
		busy = true;
		try {
			detail = await updateMongoDbResource(
				organizationId,
				projectId,
				environmentId,
				applicationId,
				{
					docker_image: image.trim(),
					public_host_port: publicPort === '' ? null : publicPort,
					runtime_cpu_cores: cpu === '' ? null : cpu,
					runtime_memory_mib: memory === '' ? null : memory,
				},
			);
			toast.success(t('projects.resource_mongodb.config_saved'));
		} catch (error) {
			toast.error(
				error instanceof Error ? error.message : t('projects.resource_mongodb.config_failed'),
			);
		} finally {
			busy = false;
		}
	}
	async function rotatePassword() {
		if (!detail || detail.deployment_status !== 'running') return;
		if (!window.confirm(t('projects.resource_mongodb.rotate_confirm'))) return;
		busy = true;
		try {
			detail = await rotateMongoDbPassword(
				organizationId,
				projectId,
				environmentId,
				applicationId,
				{},
			);
			password = detail.password ?? '';
			showPassword = false;
			showPrivateUrl = false;
			showPublicUrl = false;
			toast.success(t('projects.resource_mongodb.rotate_done'));
		} catch (error) {
			toast.error(
				error instanceof Error ? error.message : t('projects.resource_mongodb.rotate_failed'),
			);
		} finally {
			busy = false;
		}
	}
	// Re-reads the resource whenever the parent's deployment state moves, so the
	// connection details appear as soon as a deploy finishes - no page reload.
	$effect(() => {
		deploymentStatus;
		void refresh();
	});
</script>

{#snippet secretInput(
	id: string,
	value: string,
	shown: boolean,
	toggle: () => void,
	label: string,
)}<div class="grid gap-2">
		<Label for={id}>{label}</Label>
		<div class="relative">
			<Input
				{id}
				type={shown ? 'text' : 'password'}
				{value}
				readonly
				class="pr-20 font-mono text-xs" /><Button
				type="button"
				variant="ghost"
				size="icon"
				class="absolute right-9 top-0 size-9"
				onclick={toggle}
				aria-label={shown
					? t('projects.resource_mongodb.aria_hide', { label })
					: t('projects.resource_mongodb.aria_reveal', { label })}
				>{#if shown}<EyeOffIcon class="size-4" />{:else}<EyeIcon class="size-4" />{/if}</Button
			><CopyButton {value} label={null} revealOnHover={false} class="absolute right-2 top-2" />
		</div>
	</div>{/snippet}

{#if loading}<SkeletonRows rows={4} height="h-16" label={t('projects.resource_mongodb.loading')} />
{:else if detail}<div class="grid gap-6">
		{#if mode === 'overview'}
			<header
				class="lp-lifted flex flex-wrap items-center justify-between gap-3 rounded-[min(var(--radius-4xl),28px)] bg-card p-5 ring-1 ring-foreground/8">
				<div class="flex items-center gap-3">
					<MongoDbIcon class="size-9" />
					<div>
						<h2 class="text-base font-semibold tracking-tight">MongoDB</h2>
						<p class="text-sm text-muted-foreground">
							{detail.docker_image} · {detail.deployment_status}
						</p>
					</div>
				</div>
				{#if allowedHere('app.deploy')}
					<div class="flex flex-wrap items-center gap-2">
						<Button disabled={busy || runtimeBusy} onclick={() => void onDeploy()}
							>{deployLabel(detail.deployment_status === 'running', 'MongoDB')}</Button
						><InternalTemplateRuntimeActions
							status={detail.deployment_status}
							busy={runtimeBusy}
							{onStart}
							{onStop}
							{onRestart} />
					</div>
				{/if}
			</header>
		{/if}
		{#if detail.config_stale && allowedHere('app.deploy')}<div
				class="flex flex-wrap items-center justify-between gap-3 rounded-lg border border-warning/40 bg-warning/5 p-3 text-sm">
				<span>{t('projects.resource_common.config_stale')}</span><Button
					size="sm"
					disabled={busy}
					onclick={() => void onDeploy()}>{t('projects.environment.redeploy')}</Button>
			</div>{/if}
		{#if mode === 'overview'}
			<section
				class="lp-lifted rounded-[min(var(--radius-4xl),28px)] bg-card p-5 ring-1 ring-foreground/8">
				<h3 class="text-base font-semibold tracking-tight">
					{t('projects.resource_common.internal_credentials')}
				</h3>
				<p class="mt-1 text-sm text-muted-foreground">
					{t('projects.resource_mongodb.internal_hint')}
					{detail.deployment_status === 'not_deployed'
						? t('projects.resource_mongodb.edit_before_first_deploy')
						: ''}
				</p>
				<div class="mt-5 grid gap-4 md:grid-cols-2">
					<div class="grid gap-2">
						<Label>{t('projects.resource_mongodb.database_name')}</Label>
						<div class="rounded-md border bg-muted/30 px-3 py-2 font-mono">
							{detail.database_name}
						</div>
					</div>
					<div class="grid gap-2">
						<Label>{t('projects.resource_mongodb.database_user')}</Label>
						<div class="rounded-md border bg-muted/30 px-3 py-2 font-mono">
							{detail.database_user}
						</div>
					</div>
					<div class="grid gap-2">
						<Label for="mongodb-password">{t('projects.resource_mongodb.root_password')}</Label>
						<div class="relative">
							<Input
								id="mongodb-password"
								type={showPassword ? 'text' : 'password'}
								bind:value={password}
								readonly={detail.deployment_status !== 'not_deployed'}
								class="pr-20 font-mono" /><Button
								type="button"
								variant="ghost"
								size="icon"
								class="absolute right-9 top-0 size-9"
								onclick={() => (showPassword = !showPassword)}
								aria-label={t('projects.resource_mongodb.reveal_root_password')}
								>{#if showPassword}<EyeOffIcon class="size-4" />{:else}<EyeIcon
										class="size-4" />{/if}</Button
							><CopyButton
								value={password}
								label={null}
								revealOnHover={false}
								class="absolute right-2 top-2" />
						</div>
					</div>
					<div class="grid gap-2">
						<Label>{t('projects.resource_common.internal_port')}</Label>
						<div class="rounded-md border bg-muted/30 px-3 py-2 font-mono">
							{detail.internal_port}
						</div>
					</div>
					<div class="grid gap-2">
						<Label>{t('projects.resource_common.internal_host')}</Label><CopyButton
							value={detail.private_alias}
							label={detail.private_alias}
							class="rounded-md border bg-muted/30 px-3 py-2" />
					</div>
					{#if detail.private_connection_string}<div class="md:col-span-2">
							{@render secretInput(
								'mongodb-private-url',
								detail.private_connection_string,
								showPrivateUrl,
								() => (showPrivateUrl = !showPrivateUrl),
								t('projects.resource_mongodb.private_url'),
							)}
						</div>{/if}
				</div>
				{#if detail.deployment_status === 'not_deployed'}<Button
						class="mt-5"
						disabled={busy}
						onclick={() => void savePassword()}
						>{busy
							? t('projects.projects.saving')
							: t('projects.resource_mongodb.save_password')}</Button
					>{/if}
			</section>
			{#if detail.deployment_status !== 'running'}<div
					class="rounded-xl border border-dashed p-6 text-center">
					<h3 class="font-medium">{t('projects.resource_mongodb.deploy_heading')}</h3>
					<p class="mt-1 text-sm text-muted-foreground">
						{t('projects.resource_mongodb.deploy_hint')}
					</p>
				</div>{/if}
			<section
				class="lp-lifted rounded-[min(var(--radius-4xl),28px)] bg-card p-5 ring-1 ring-foreground/8">
				<h3 class="text-base font-semibold tracking-tight">
					{t('projects.resource_common.external_access')}
				</h3>
				<p class="mt-1 text-sm text-muted-foreground">
					{t('projects.resource_mongodb.external_hint')}
				</p>
				<form
					class="mt-5"
					onsubmit={(event) => {
						event.preventDefault();
						void savePublicPort();
					}}>
					<div class="grid gap-2 md:max-w-xs">
						<Label for="mongodb-port">{t('projects.resource_common.public_host_port')}</Label><Input
							id="mongodb-port"
							type="number"
							min="1"
							max="65535"
							bind:value={publicPort}
							placeholder={t('projects.resource_common.not_public')} />
					</div>
					<Button class="mt-5" type="submit" disabled={busy}
						>{busy
							? t('projects.projects.saving')
							: t('projects.resource_common.save_public_access')}</Button>
				</form>
				{#if detail.public_host_port && !detail.public_connection_string}<p
						class="mt-4 text-sm text-muted-foreground">
						{t('projects.resource_mongodb.pending_hint')}
					</p>{/if}
			</section>
			{#if detail.public_connection_string}<section
					class="lp-lifted rounded-[min(var(--radius-4xl),28px)] bg-card p-5 ring-1 ring-foreground/8">
					<h3 class="text-base font-semibold tracking-tight">
						{t('projects.resource_mongodb.external_credentials')}
					</h3>
					<p class="mt-1 text-sm text-muted-foreground">
						{t('projects.resource_mongodb.direct_tcp', { port: detail.public_host_port })}
					</p>
					<div class="mt-5 grid gap-4">
						{@render secretInput(
							'mongodb-public-url',
							detail.public_connection_string,
							showPublicUrl,
							() => (showPublicUrl = !showPublicUrl),
							t('projects.resource_mongodb.public_url'),
						)}
					</div>
				</section>{/if}
			{#if detail.deployment_status === 'running'}<Button
					variant="outline"
					disabled={busy}
					onclick={() => void rotatePassword()}
					>{t('projects.resource_mongodb.rotate_password')}</Button
				>{/if}
			<section
				class="lp-lifted rounded-[min(var(--radius-4xl),28px)] bg-card p-5 ring-1 ring-foreground/8">
				<ApplicationComposeTab
					{busy}
					{composeContent}
					{composePortMappings}
					{runtimeVariableNames}
					onSave={onComposeSave}
					onDeploy={onComposeDeploy}
					onValidate={onComposeValidate}
					onContentChange={onComposeContentChange}
					{onCreateRuntimeVariables}
					running={detail.deployment_status === 'running'}
					readOnly
					showPortMappings={false}
					editWarning={t('projects.resource_mongodb.edit_warning')} />
			</section>
		{:else}<section
				class="lp-lifted rounded-[min(var(--radius-4xl),28px)] bg-card p-5 ring-1 ring-foreground/8">
				<h3 class="text-base font-semibold tracking-tight">
					{t('projects.resource_mongodb.config_title')}
				</h3>
				<p class="mt-1 text-sm text-muted-foreground">
					{t('projects.resource_common.changes_effect')}
				</p>
				<form
					class="mt-5"
					onsubmit={(event) => {
						event.preventDefault();
						void saveConfiguration();
					}}>
					<div class="grid gap-4 md:grid-cols-2">
						<div class="grid gap-2 md:col-span-2">
							<Label for="mongodb-image">{t('projects.resource_common.docker_image')}</Label><Input
								id="mongodb-image"
								bind:value={image} />
						</div>
						<div class="grid gap-2">
							<Label for="mongodb-cpu">{t('projects.resource_common.cpu_cores')}</Label><Input
								id="mongodb-cpu"
								type="number"
								step="0.1"
								bind:value={cpu}
								placeholder={t('projects.resource_common.unlimited')} />
						</div>
						<div class="grid gap-2">
							<Label for="mongodb-memory">{t('projects.resource_common.memory_mib')}</Label><Input
								id="mongodb-memory"
								type="number"
								min="1"
								bind:value={memory}
								placeholder={t('projects.resource_common.unlimited')} />
						</div>
					</div>
					<Button class="mt-5" type="submit" disabled={busy}
						>{busy
							? t('projects.projects.saving')
							: t('projects.resource_common.save_configuration')}</Button>
				</form>
			</section>{/if}
	</div>
	<RedeployConfirmDialog bind:open={redeployOpen} port={redeployPort} ondeploy={onDeploy} />{/if}
