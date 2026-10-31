<script lang="ts">
	/**
	 * Requires two-factor authentication from every member of one organization.
	 * A member without it is refused by the API until they set it up on their
	 * account, so the panel says who that would hit before the switch is flipped.
	 */
	import { toast } from '#lib/utils/toast.js';
	import { Label } from '#lib/components/ui/label/index.js';
	import SoftSwitch from '#lib/components/custom/soft-switch.svelte';
	import {
		updateOrganization,
		type OrganizationDto,
		type OrganizationMemberDto,
	} from '#lib/clients/organization-client.js';
	import { t } from '#lib/i18n/index.svelte.js';

	let {
		organizationId,
		organization = $bindable(),
		members,
		myUserId,
		isSysadmin,
	}: {
		organizationId: string;
		organization: OrganizationDto | null;
		members: OrganizationMemberDto[];
		myUserId: string | null;
		isSysadmin: boolean;
	} = $props();

	// Tab names live on the account page, which reads `?tab=`.
	const securityHref = '/account?tab=security';

	let saving = $state(false);

	const withoutTwoFactor = $derived(members.filter((member) => member.two_factor_enabled === false));
	/** The API refuses to turn the requirement on for an admin who has no second
	 *  factor (they would lock themselves out); say so up front. The SysAdmin is
	 *  exempt from the requirement, so is not held back. */
	const ownMissing = $derived(
		!isSysadmin &&
			members.find((member) => member.user_id === myUserId)?.two_factor_enabled === false,
	);

	async function toggle(checked: boolean) {
		if (!organization) return;
		const previous = organization.require_two_factor;
		organization.require_two_factor = checked;
		saving = true;
		try {
			const updated = await updateOrganization(organizationId, { require_two_factor: checked });
			organization.require_two_factor = updated.require_two_factor;
		} catch (error) {
			organization.require_two_factor = previous;
			toast.error(error instanceof Error ? error.message : t('organizations.two_factor.update_failed'));
		} finally {
			saving = false;
		}
	}
</script>

<div class="mt-4 flex flex-col gap-4">
	<div
		class="lp-raised bg-card flex flex-col gap-3 rounded-2xl border p-4 sm:flex-row sm:items-center sm:justify-between sm:gap-4">
		<div class="max-w-[70ch]">
			<Label for="require-two-factor">{t('organizations.two_factor.title')}</Label>
			<p class="text-muted-foreground mt-1 text-sm">{t('organizations.two_factor.description')}</p>
			{#if ownMissing && !organization?.require_two_factor}
				<p class="mt-2 text-sm text-amber-600 dark:text-amber-400">
					{t('organizations.two_factor.own_first')}
					<a class="underline" href={securityHref}>{t('organizations.two_factor.own_first_link')}</a>
				</p>
			{/if}
		</div>
		<SoftSwitch
			id="require-two-factor"
			checked={organization?.require_two_factor ?? false}
			disabled={saving || !organization || (ownMissing && !organization?.require_two_factor)}
			onCheckedChange={toggle} />
	</div>

	<p class="text-muted-foreground text-sm">
		{#if withoutTwoFactor.length > 0}
			{t('organizations.two_factor.missing', {
				count: withoutTwoFactor.length,
				total: members.length,
			})}
		{:else if members.length > 0}
			{t('organizations.two_factor.all_set')}
		{/if}
	</p>
</div>
