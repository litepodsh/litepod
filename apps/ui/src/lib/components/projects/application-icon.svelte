<script lang="ts">
	import PostgresIcon from '#lib/components/icons/postgres-icon.svelte';
	import MariaDbIcon from '#lib/components/icons/mariadb-icon.svelte';
	import DragonflyDbIcon from '#lib/components/icons/dragonflydb-icon.svelte';
	import KeyDbIcon from '#lib/components/icons/keydb-icon.svelte';
	import SqlServerIcon from '#lib/components/icons/sqlserver-icon.svelte';
	import MongoDbIcon from '#lib/components/icons/mongodb-icon.svelte';
	import BoxesIcon from '@lucide/svelte/icons/boxes';
	import { displayApplicationIconUrl } from '#lib/utils/application-card-icon.js';

	let {
		templateKind,
		catalogIconUrl,
		iconUrl = null,
		applicationId,
		failedApplicationIds = $bindable(),
		class: className = '',
	}: {
		templateKind: string | null;
		catalogIconUrl: string | null;
		/** User-uploaded or pasted override; takes precedence over `catalogIconUrl`. */
		iconUrl?: string | null;
		applicationId: string;
		failedApplicationIds: Set<string>;
		class?: string;
	} = $props();

	let iconClass = $derived(className || 'size-5');
	let displayUrl = $derived(
		displayApplicationIconUrl(applicationId, iconUrl, catalogIconUrl, failedApplicationIds),
	);
</script>

{#if displayUrl}
	<img
		src={displayUrl}
		alt=""
		loading="lazy"
		class={`object-contain ${className || 'size-6'}`}
		onerror={() => {
			failedApplicationIds = new Set([...failedApplicationIds, applicationId]);
		}} />
{:else if templateKind === 'postgres'}
	<PostgresIcon class={iconClass} />
{:else if templateKind === 'mariadb'}
	<MariaDbIcon class={iconClass} />
{:else if templateKind === 'dragonflydb'}
	<DragonflyDbIcon class={iconClass} />
{:else if templateKind === 'keydb'}
	<KeyDbIcon class={iconClass} />
{:else if templateKind === 'sqlserver'}
	<SqlServerIcon class={iconClass} />
{:else if templateKind === 'mongodb'}
	<MongoDbIcon class={iconClass} />
{:else}
	<BoxesIcon class={iconClass} />
{/if}
