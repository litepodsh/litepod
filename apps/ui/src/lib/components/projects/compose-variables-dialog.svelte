<script lang="ts">
	import * as Dialog from '#lib/components/ui/dialog/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import SoftCheckbox from '#lib/components/custom/soft-checkbox.svelte';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Label } from '#lib/components/ui/label/index.js';
	import { toast } from '#lib/utils/toast.js';
	import type { ComposeVarRef } from '#lib/utils/compose-variables.js';
	import { t } from '#lib/i18n/index.svelte.js';

	let {
		open = $bindable(false),
		variables = [],
		onCreate,
	}: {
		open?: boolean;
		variables?: ComposeVarRef[];
		onCreate: (created: { name: string; value: string }[]) => Promise<void>;
	} = $props();

	type Row = { name: string; value: string; include: boolean; creatable: boolean };

	let rows = $state<Row[]>([]);
	let saving = $state(false);
	let wasOpen = $state(false);

	$effect(() => {
		if (open && !wasOpen) {
			rows = variables.map((variable) => ({
				name: variable.name,
				value: '',
				include: variable.creatable,
				creatable: variable.creatable,
			}));
		}
		wasOpen = open;
	});

	let selected = $derived(rows.filter((row) => row.include && row.creatable));

	async function submit() {
		if (selected.length === 0) return;
		saving = true;
		try {
			await onCreate(selected.map(({ name, value }) => ({ name, value })));
			open = false;
		} catch (error) {
			toast.error(
				error instanceof Error ? error.message : t('projects.compose_variables.create_failed'),
			);
		} finally {
			saving = false;
		}
	}
</script>

<Dialog.Root bind:open>
	<Dialog.Content>
		<Dialog.Header>
			<Dialog.Title>{t('projects.compose_variables.title')}</Dialog.Title>
			<Dialog.Description>
				{t('projects.compose_variables.hint')}
			</Dialog.Description>
		</Dialog.Header>
		<form
			class="grid gap-3"
			onsubmit={(event) => {
				event.preventDefault();
				void submit();
			}}>
			{#each rows as row, i (row.name)}
				<div class="grid gap-1.5">
					<div class="flex items-center gap-3">
						<SoftCheckbox
							id="compose-variable-{i}"
							bind:checked={rows[i].include}
							disabled={saving || !row.creatable}
							aria-label={t('projects.compose_variables.create_named', { name: row.name })} />
						<Label
							for="compose-variable-{i}"
							class="w-52 shrink-0 font-mono text-xs {row.creatable
								? ''
								: 'text-muted-foreground'}">
							{row.name}
						</Label>
						<Input
							bind:value={rows[i].value}
							placeholder={t('projects.compose_variables.value')}
							disabled={saving || !row.creatable || !row.include}
							aria-label={t('projects.compose_variables.value_for', { name: row.name })} />
					</div>
					{#if !row.creatable}
						<p class="pl-7 text-xs text-muted-foreground">
							{t('projects.compose_variables.invalid_name_hint')}
						</p>
					{/if}
				</div>
			{/each}
			<Dialog.Footer>
				<Button type="button" variant="outline" onclick={() => (open = false)} disabled={saving}>
					{t('projects.projects.cancel')}
				</Button>
				<Button type="submit" disabled={saving || selected.length === 0}>
					{saving
						? t('projects.volume.creating')
						: t('projects.compose_variables.create_count', { count: selected.length })}
				</Button>
			</Dialog.Footer>
		</form>
	</Dialog.Content>
</Dialog.Root>
