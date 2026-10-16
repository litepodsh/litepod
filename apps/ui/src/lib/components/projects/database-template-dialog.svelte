<script lang="ts">
	import * as Dialog from '#lib/components/ui/dialog/index.js';
	import DialogScrollContent from '#lib/components/custom/dialog-scroll-content.svelte';
	import type { ApplicationDto, ServerSummaryDto } from '#lib/clients/project-client.js';
	import { Label } from '#lib/components/ui/label/index.js';
	import * as Select from '#lib/components/ui/select/index.js';
	import PostgresTemplateDialog from '#lib/components/projects/postgres-template-dialog.svelte';
	import MariaDbTemplateDialog from '#lib/components/projects/mariadb-template-dialog.svelte';
	import DragonflyDbTemplateDialog from '#lib/components/projects/dragonflydb-template-dialog.svelte';
	import KeyDbTemplateDialog from '#lib/components/projects/keydb-template-dialog.svelte';
	import SqlServerTemplateDialog from '#lib/components/projects/sqlserver-template-dialog.svelte';
	import MongoDbTemplateDialog from '#lib/components/projects/mongodb-template-dialog.svelte';
	import PostgresIcon from '#lib/components/icons/postgres-icon.svelte';
	import MariaDbIcon from '#lib/components/icons/mariadb-icon.svelte';
	import DragonflyDbIcon from '#lib/components/icons/dragonflydb-icon.svelte';
	import KeyDbIcon from '#lib/components/icons/keydb-icon.svelte';
	import SqlServerIcon from '#lib/components/icons/sqlserver-icon.svelte';
	import MongoDbIcon from '#lib/components/icons/mongodb-icon.svelte';
	import { t } from '#lib/i18n/index.svelte.js';

	type DatabaseTemplate =
		| 'catalog'
		| 'postgres'
		| 'mariadb'
		| 'dragonflydb'
		| 'keydb'
		| 'sqlserver'
		| 'mongodb';
	type Created = { application: ApplicationDto };

	let {
		open = $bindable(false),
		organizationId,
		projectId,
		environmentId,
		servers = [],
		onPostgresCreated,
		onMariaDbCreated,
		onDragonflyDbCreated,
		onKeyDbCreated,
		onSqlServerCreated,
		onMongoDbCreated,
	}: {
		open?: boolean;
		organizationId: string;
		projectId: string;
		environmentId: string;
		servers?: ServerSummaryDto[];
		onPostgresCreated: (created: Created) => Promise<void>;
		onMariaDbCreated: (created: Created) => Promise<void>;
		onDragonflyDbCreated: (created: Created) => Promise<void>;
		onKeyDbCreated: (created: Created) => Promise<void>;
		onSqlServerCreated: (created: Created) => Promise<void>;
		onMongoDbCreated: (created: Created) => Promise<void>;
	} = $props();

	let selected = $state<DatabaseTemplate>('catalog');
	let serverId = $state('');

	const serverTriggerLabel = $derived.by(() => {
		const match = servers.find((server) => server.id === serverId);
		return match
			? `${match.name} · ${match.ingress_mode.replace('_', ' ')}`
			: t('projects.template_common.choose_ready_server');
	});

	function close() {
		open = false;
	}

	function returnToCatalog() {
		selected = 'catalog';
	}

	$effect(() => {
		if (open && !serverId && servers.length > 0) {
			serverId =
				servers.length === 1
					? servers[0].id
					: (servers.find((s) => s.name.toLowerCase().includes('local'))?.id ?? '');
		}
		if (!open) {
			selected = 'catalog';
			serverId = '';
		}
	});
</script>

<Dialog.Root bind:open>
	<DialogScrollContent class="sm:max-w-3xl">
		{#if selected === 'catalog'}
			<Dialog.Header>
				<Dialog.Title>{t('projects.template_catalog.create_database')}</Dialog.Title>
				<Dialog.Description
					>{t('projects.template_catalog.select_template_hint')}</Dialog.Description>
			</Dialog.Header>
			<div class="grid gap-2">
				<Label for="database-template-server">{t('organizations.common.server')}</Label>
				<Select.Root type="single" bind:value={serverId}>
					<Select.Trigger id="database-template-server" class="w-full"
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
			<div class="grid gap-3 sm:grid-cols-2">
				<button
					type="button"
					class="flex items-center gap-3 rounded-xl border p-4 text-left transition-colors hover:border-primary/50 hover:bg-accent"
					onclick={() => (selected = 'postgres')}
					><PostgresIcon class="size-7 text-primary" /><span
						><span class="block font-medium">PostgreSQL</span><span
							class="text-sm text-muted-foreground"
							>{t('projects.template_common.relational_database')}</span
						></span
					></button>
				<button
					type="button"
					class="flex items-center gap-3 rounded-xl border p-4 text-left transition-colors hover:border-primary/50 hover:bg-accent"
					onclick={() => (selected = 'mariadb')}
					><MariaDbIcon class="size-7" /><span
						><span class="block font-medium">MariaDB</span><span
							class="text-sm text-muted-foreground"
							>{t('projects.template_common.relational_database')}</span
						></span
					></button>
				<button
					type="button"
					class="flex items-center gap-3 rounded-xl border p-4 text-left transition-colors hover:border-primary/50 hover:bg-accent"
					onclick={() => (selected = 'dragonflydb')}
					><DragonflyDbIcon class="size-7" /><span
						><span class="block font-medium">DragonflyDB</span><span
							class="text-sm text-muted-foreground"
							>{t('projects.template_common.redis_compatible_database')}</span
						></span
					></button>
				<button
					type="button"
					class="flex items-center gap-3 rounded-xl border p-4 text-left transition-colors hover:border-primary/50 hover:bg-accent"
					onclick={() => (selected = 'keydb')}
					><KeyDbIcon class="size-7" /><span
						><span class="block font-medium">KeyDB</span><span class="text-sm text-muted-foreground"
							>{t('projects.template_common.redis_compatible_database')}</span
						></span
					></button>
				<button
					type="button"
					class="flex items-center gap-3 rounded-xl border p-4 text-left transition-colors hover:border-primary/50 hover:bg-accent"
					onclick={() => (selected = 'sqlserver')}
					><SqlServerIcon class="size-7" /><span
						><span class="block font-medium"
							>{t('projects.template_catalog.mssql_developer_edition')}</span
						><span class="text-sm text-muted-foreground"
							>{t('projects.template_catalog.microsoft_sql_server')}</span
						></span
					></button>
				<button
					type="button"
					class="flex items-center gap-3 rounded-xl border p-4 text-left transition-colors hover:border-primary/50 hover:bg-accent"
					onclick={() => (selected = 'mongodb')}
					><MongoDbIcon class="size-7" /><span
						><span class="block font-medium">MongoDB</span><span
							class="text-sm text-muted-foreground"
							>{t('projects.template_common.document_database')}</span
						></span
					></button>
			</div>
		{:else if selected === 'postgres'}
			<PostgresTemplateDialog
				{organizationId}
				{projectId}
				{environmentId}
				{servers}
				{serverId}
				onCreated={onPostgresCreated}
				onBack={returnToCatalog}
				onCancel={close} />
		{:else if selected === 'mariadb'}
			<MariaDbTemplateDialog
				{organizationId}
				{projectId}
				{environmentId}
				{servers}
				{serverId}
				onCreated={onMariaDbCreated}
				onBack={returnToCatalog}
				onCancel={close} />
		{:else if selected === 'dragonflydb'}
			<DragonflyDbTemplateDialog
				{organizationId}
				{projectId}
				{environmentId}
				{servers}
				{serverId}
				onCreated={onDragonflyDbCreated}
				onBack={returnToCatalog}
				onCancel={close} />
		{:else if selected === 'keydb'}
			<KeyDbTemplateDialog
				{organizationId}
				{projectId}
				{environmentId}
				{servers}
				{serverId}
				onCreated={onKeyDbCreated}
				onBack={returnToCatalog}
				onCancel={close} />
		{:else if selected === 'sqlserver'}
			<SqlServerTemplateDialog
				{organizationId}
				{projectId}
				{environmentId}
				{servers}
				{serverId}
				onCreated={onSqlServerCreated}
				onBack={returnToCatalog}
				onCancel={close} />
		{:else if selected === 'mongodb'}
			<MongoDbTemplateDialog
				{organizationId}
				{projectId}
				{environmentId}
				{servers}
				{serverId}
				onCreated={onMongoDbCreated}
				onBack={returnToCatalog}
				onCancel={close} />
		{/if}
	</DialogScrollContent>
</Dialog.Root>
