<script lang="ts">
	/**
	 * The MCP server's switch for one organization, plus what to paste into a
	 * client. Off is the real off: the endpoint refuses every call while this
	 * is false.
	 */
	import { toast } from '#lib/utils/toast.js';
	import { Label } from '#lib/components/ui/label/index.js';
	import SoftSwitch from '#lib/components/custom/soft-switch.svelte';
	import CopyButton from '#lib/components/custom/copy-button.svelte';
	import { API_URL } from '#lib/clients/base-client.js';
	import { updateOrganization, type OrganizationDto } from '#lib/clients/organization-client.js';
	import { t } from '#lib/i18n/index.svelte.js';

	let {
		organizationId,
		organization = $bindable(),
	}: { organizationId: string; organization: OrganizationDto | null } = $props();

	// Tokens are made on the account page; it reads its tab from `?tab=`.
	const tokensHref = '/account?tab=tokens';

	let saving = $state(false);

	// API_URL is empty when the UI and the API share an origin.
	const endpoint = $derived(
		new URL(
			`/api/v1/organizations/${organizationId}/mcp`,
			API_URL || window.location.origin,
		).toString(),
	);
	const snippet = $derived(
		`claude mcp add --transport http litepod ${endpoint} --header "Authorization: Bearer <token>"`,
	);

	async function toggle(checked: boolean) {
		if (!organization) return;
		const previous = organization.mcp_enabled;
		organization.mcp_enabled = checked;
		saving = true;
		try {
			const updated = await updateOrganization(organizationId, { mcp_enabled: checked });
			organization.mcp_enabled = updated.mcp_enabled;
		} catch (error) {
			organization.mcp_enabled = previous;
			toast.error(error instanceof Error ? error.message : t('organizations.mcp.update_failed'));
		} finally {
			saving = false;
		}
	}
</script>

<div class="mt-4 flex flex-col gap-4">
	<div
		class="lp-raised bg-card flex flex-col gap-3 rounded-2xl border p-4 sm:flex-row sm:items-center sm:justify-between sm:gap-4">
		<div class="max-w-[70ch]">
			<Label for="mcp-enabled">{t('organizations.mcp.title')}</Label>
			<p class="text-muted-foreground mt-1 text-sm">{t('organizations.mcp.description')}</p>
		</div>
		<SoftSwitch
			id="mcp-enabled"
			checked={organization?.mcp_enabled ?? false}
			disabled={saving || !organization}
			onCheckedChange={toggle} />
	</div>

	{#if organization?.mcp_enabled}
		<div class="lp-raised bg-card flex flex-col gap-3 rounded-2xl border p-4">
			<div class="min-w-0">
				<p class="text-sm font-medium">{t('organizations.mcp.endpoint')}</p>
				<CopyButton
					value={endpoint}
					revealOnHover={false}
					class="mt-1 font-mono text-xs break-all" />
			</div>
			<div class="min-w-0">
				<p class="text-sm font-medium">{t('organizations.mcp.snippet')}</p>
				<CopyButton
					value={snippet}
					revealOnHover={false}
					class="mt-1 font-mono text-xs break-all" />
			</div>
			<p class="text-muted-foreground text-sm">
				{t('organizations.mcp.token_hint')}
				<a class="underline" href={tokensHref}>{t('organizations.mcp.create_token')}</a>
			</p>
		</div>
	{/if}
</div>
