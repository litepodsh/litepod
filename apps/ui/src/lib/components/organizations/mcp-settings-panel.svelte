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
	import MonacoCodeBlock from '#lib/components/assistant/monaco-code-block.svelte';
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
	// `-s user` so the server is there in every folder, not only the one it was added from.
	// OAuth-first: pasting the endpoint opens Litepod to sign in; no token.
	const claudeCode = $derived(`claude mcp add -s user --transport http litepod ${endpoint}`);
	const desktop = $derived(
		JSON.stringify(
			{
				mcpServers: {
					litepod: {
						command: 'npx',
						args: ['-y', 'mcp-remote', endpoint, '--header', 'Authorization:${LITEPOD_AUTH}'],
						env: { LITEPOD_AUTH: 'Bearer <token>' },
					},
				},
			},
			null,
			2,
		),
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
				<p class="text-muted-foreground mt-0.5 text-xs">{t('organizations.mcp.endpoint_hint')}</p>
				<div class="lp-well relative mt-1.5 rounded-lg bg-muted/60 p-3 pr-10">
					<code class="block font-mono text-xs leading-relaxed break-all">{endpoint}</code>
					<CopyButton
						value={endpoint}
						label={null}
						revealOnHover={false}
						class="absolute top-1.5 right-1.5 size-7 justify-center rounded-md text-muted-foreground hover:bg-card hover:text-foreground" />
				</div>
			</div>
			<div class="min-w-0">
				<p class="text-sm font-medium">{t('organizations.mcp.snippet')}</p>
				<p class="text-muted-foreground mt-0.5 text-xs">{t('organizations.mcp.snippet_hint')}</p>
				<div class="relative mt-1.5">
					<MonacoCodeBlock value={claudeCode} language="shell" class="pr-10" />
					<CopyButton
						value={claudeCode}
						label={null}
						revealOnHover={false}
						class="absolute top-1.5 right-1.5 size-7 justify-center rounded-md text-muted-foreground hover:bg-card hover:text-foreground" />
				</div>
			</div>
			<p class="text-muted-foreground text-sm">{t('organizations.mcp.chatgpt_hint')}</p>
			<div class="mt-2">
				<p class="text-sm font-medium text-muted-foreground">{t('organizations.mcp.without_oauth')}</p>
			</div>
			<div class="min-w-0">
				<p class="text-sm font-medium">{t('organizations.mcp.desktop')}</p>
				<p class="text-muted-foreground mt-0.5 text-xs">{t('organizations.mcp.desktop_hint')}</p>
				<div class="relative mt-1.5">
					<MonacoCodeBlock value={desktop} language="json" class="pr-10" />
					<CopyButton
						value={desktop}
						label={null}
						revealOnHover={false}
						class="absolute top-1.5 right-1.5 size-7 justify-center rounded-md text-muted-foreground hover:bg-card hover:text-foreground" />
				</div>
			</div>
			<p class="text-muted-foreground text-sm">
				{t('organizations.mcp.token_hint')}
				<a class="underline" href={tokensHref}>{t('organizations.mcp.create_token')}</a>
			</p>
		</div>
	{/if}
</div>
