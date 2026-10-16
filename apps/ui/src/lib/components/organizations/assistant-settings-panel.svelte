<script lang="ts">
	/**
	 * The assistant's kill switch for one organization.
	 *
	 * Off is the real off: the API refuses every assistant route while this is
	 * false, and the sidebar hides the switch entirely, so nobody discovers a
	 * feature that would only tell them it is unavailable.
	 */
	import { toast } from '#lib/utils/toast.js';
	import { Label } from '#lib/components/ui/label/index.js';
	import SoftSwitch from '#lib/components/custom/soft-switch.svelte';
	import { updateOrganization, type OrganizationDto } from '#lib/clients/organization-client.js';
	import AssistantConnectionsPanel from './assistant-connections-panel.svelte';
	import { t } from '#lib/i18n/index.svelte.js';

	let {
		organizationId,
		organization = $bindable(),
	}: { organizationId: string; organization: OrganizationDto | null } = $props();

	let saving = $state(false);

	async function toggle(checked: boolean) {
		if (!organization) return;
		const previous = organization.assistant_enabled;
		organization.assistant_enabled = checked;
		saving = true;
		try {
			const updated = await updateOrganization(organizationId, { assistant_enabled: checked });
			organization.assistant_enabled = updated.assistant_enabled;
		} catch (error) {
			organization.assistant_enabled = previous;
			toast.error(
				error instanceof Error ? error.message : t('organizations.assistant.update_failed'),
			);
		} finally {
			saving = false;
		}
	}
</script>

<div class="flex flex-col gap-4">
	<div
		class="lp-raised bg-card flex flex-col gap-3 rounded-2xl border p-4 sm:flex-row sm:items-center sm:justify-between sm:gap-4">
		<div class="max-w-[70ch]">
			<Label for="assistant-enabled">{t('organizations.assistant.title')}</Label>
			<p class="text-muted-foreground mt-1 text-sm">
				{t('organizations.assistant.description')}
			</p>
		</div>
		<SoftSwitch
			id="assistant-enabled"
			checked={organization?.assistant_enabled ?? false}
			disabled={saving || !organization}
			onCheckedChange={toggle} />
	</div>

	<AssistantConnectionsPanel {organizationId} />

	<div class="lp-raised bg-card rounded-2xl border p-4">
		<p class="text-sm font-medium">{t('organizations.assistant.can_see')}</p>
		<ul class="text-muted-foreground mt-2 grid gap-1 text-sm">
			<li>{t('organizations.assistant.see_projects')}</li>
			<li>
				{t('organizations.assistant.see_env_names_before')}<em
					>{t('organizations.assistant.see_env_names_word')}</em
				>{t('organizations.assistant.see_env_names_after')}
			</li>
			<li>{t('organizations.assistant.see_deployments')}</li>
			<li>{t('organizations.assistant.see_compose')}</li>
		</ul>
		<p class="text-muted-foreground mt-3 text-sm">
			{t('organizations.assistant.scrubbed')}
		</p>
	</div>
</div>
