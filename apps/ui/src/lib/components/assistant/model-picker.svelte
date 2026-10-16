<script lang="ts">
	/**
	 * Provider and model in one control.
	 *
	 * Every model the organization's connections report is listed, including the
	 * ones this build cannot run — those carry the reason and are not selectable.
	 * Hiding them would leave the user hunting for something that is right there
	 * in the provider's own list.
	 *
	 * One search field covers both levels: the item value starts with the
	 * connection's name, so typing `ollama` narrows to that provider and typing
	 * `coder` matches across all of them.
	 */
	import SearchableDropdown, {
		type SearchableDropdownItem,
	} from '#lib/components/custom/searchable-dropdown.svelte';
	import { listConnections, type ConnectionDto } from '#lib/clients/assistant-connection-client.js';
	import { chooseModel, thread } from './thread.svelte';
	import { t } from '#lib/i18n/index.svelte.js';

	let { organizationId }: { organizationId: string | null } = $props();

	let connections = $state<ConnectionDto[]>([]);

	$effect(() => {
		if (!organizationId) return;
		let live = true;
		listConnections(organizationId)
			.then((loaded) => {
				if (live) connections = loaded;
			})
			.catch(() => {
				// The picker is a convenience: with no list it falls back to the
				// organization's default, which is what an empty trigger means.
			});
		return () => {
			live = false;
		};
	});

	const items = $derived<SearchableDropdownItem[]>(
		connections.flatMap((connection) =>
			connection.models.map((model) => ({
				value: `${connection.id}:${model.id}`,
				label: model.id,
				group: connection.name,
				disabled: model.blocked !== null,
				badge: model.blocked ?? (model.tools ? 'tools' : undefined),
				description:
					model.blocked === 'endpoint'
						? t('common.assistant.model_blocked_endpoint')
						: model.blocked === 'no-tools'
							? t('common.assistant.model_blocked_no_tools')
							: model.context
								? `${Math.round(model.context / 1000)}K context`
								: model.label === model.id
									? undefined
									: model.label,
			})),
		),
	);
</script>

<!-- Bound through `chooseModel` so the selection lands in one place: the thread
     state, which persists it and hands it to the next turn. -->
<SearchableDropdown
	{items}
	bind:selectedValue={() => thread.model, (selection) => chooseModel(selection)}
	label={t('common.assistant.model_default')}
	variant="ghost"
	size="sm"
	searchPlaceholder={t('common.assistant.model_search')}
	emptyText={t('common.assistant.model_empty')}
	disabled={items.length === 0}
	triggerClass="min-w-0 max-w-[60%] font-mono text-xs" />
