<script lang="ts">
	import * as Card from '#lib/components/ui/card/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import BugIcon from '@lucide/svelte/icons/bug';
	import ReceiptIcon from '@lucide/svelte/icons/receipt';
	import WalletIcon from '@lucide/svelte/icons/wallet';
	import UsersIcon from '@lucide/svelte/icons/users';
	import ShieldAlertIcon from '@lucide/svelte/icons/shield-alert';
	import ArrowRightIcon from '@lucide/svelte/icons/arrow-right';
	import { t } from '#lib/i18n/index.svelte.js';

	const items = [
		{
			titleKey: 'operations.admin.users_title',
			descriptionKey: 'operations.admin.users_description',
			href: '/admin/users',
			Icon: UsersIcon,
		},
		{
			titleKey: 'operations.admin.security_title',
			descriptionKey: 'operations.admin.security_description',
			href: '/admin/security',
			Icon: ShieldAlertIcon,
		},
		{
			titleKey: 'operations.admin.debug_title',
			descriptionKey: 'operations.admin.debug_description',
			href: '/debug',
			Icon: BugIcon,
		},
		{
			titleKey: 'operations.admin.payments_title',
			descriptionKey: 'operations.admin.payments_description',
			href: '/admin/billing',
			Icon: WalletIcon,
		},
		{
			titleKey: 'operations.admin.billing_title',
			descriptionKey: 'operations.admin.billing_description',
			href: '/debug/billing',
			Icon: ReceiptIcon,
		},
	];
</script>

<div class="flex min-w-0 flex-col gap-5 px-4 py-6 sm:px-6">
	<header>
		<h1 class="text-xl font-semibold tracking-tight">{t('operations.admin.title')}</h1>
		<p class="text-sm text-muted-foreground">{t('operations.admin.description')}</p>
	</header>

	<section class="space-y-3">
		<div class="grid gap-4 sm:grid-cols-2 xl:grid-cols-3">
			{#each items as item (item.href)}
				<Card.Root>
					<Card.Header class="flex-row items-start gap-3 space-y-0">
						<div class="rounded-2xl bg-primary/10 p-3 text-primary">
							<item.Icon class="size-5" />
						</div>
						<div>
							<Card.Title>{t(item.titleKey)}</Card.Title>
							<Card.Description class="mt-1">{t(item.descriptionKey)}</Card.Description>
						</div>
					</Card.Header>
					<Card.Footer>
						<Button href={item.href} class="w-full" variant="secondary">
							{t('operations.admin.open', { name: t(item.titleKey) })}
							<ArrowRightIcon />
						</Button>
					</Card.Footer>
				</Card.Root>
			{/each}
		</div>
	</section>
</div>
