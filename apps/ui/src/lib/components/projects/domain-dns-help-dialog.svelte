<script lang="ts">
	import * as Dialog from '#lib/components/ui/dialog/index.js';
	import * as Table from '#lib/components/ui/table/index.js';
	import CopyButton from '#lib/components/custom/copy-button.svelte';
	import type { DomainDto } from '#lib/clients/project-client.js';
	import type { Server } from '#lib/clients/server-client.js';
	import { t } from '#lib/i18n/index.svelte.js';

	let {
		open = $bindable(false),
		server,
		domains = [],
	}: {
		open?: boolean;
		server: Pick<Server, 'kind' | 'ingress_mode'> & { public_address?: string | null };
		domains?: DomainDto[];
	} = $props();

	const publicDomain = $derived(domains.find((domain) => !domain.is_local));
	const target = $derived.by(() => {
		if (server.ingress_mode === 'public_edge')
			return server.public_address?.trim() || publicDomain?.required_dns_target || null;
		return publicDomain?.required_dns_target ?? null;
	});
	function dnsRecordType(value: string) {
		if (/^(?:\d{1,3}\.){3}\d{1,3}$/.test(value)) return 'A';
		if (value.includes(':') && /^[\da-f:]+$/i.test(value)) return 'AAAA';
		return 'CNAME';
	}
	const targetRecordType = $derived(target ? dnsRecordType(target) : 'CNAME');
	const canConfigurePublicDns = $derived(server.ingress_mode !== 'private_only');
</script>

<Dialog.Root bind:open>
	<Dialog.Content class="sm:max-w-lg">
		<Dialog.Header>
			<Dialog.Title>{t('projects.domain_dns.title')}</Dialog.Title>
			<Dialog.Description>
				{t('projects.domain_dns.description')}
			</Dialog.Description>
		</Dialog.Header>

		{#if !canConfigurePublicDns}
			<div class="rounded-lg border border-warning/40 bg-warning/5 p-3 text-sm">
				{t('projects.domain_dns.not_permitted')}
			</div>
		{:else}
			<div class="grid gap-3">
				{#if server.ingress_mode === 'public_edge'}
					<p class="text-sm text-muted-foreground">
						{t('projects.domain_dns.dns_direct')}
					</p>
				{:else}
					<p class="text-sm text-muted-foreground">
						{t('projects.domain_dns.dns_relayed')}
					</p>
				{/if}

				{#if target}
					<div class="rounded-lg border p-3">
						<p class="text-xs font-medium text-muted-foreground">
							{t('projects.domain_dns.dns_target')}
						</p>
						<div class="mt-1 flex items-center justify-between gap-3">
							<code class="break-all font-mono text-sm">{target}</code>
							<CopyButton
								value={target}
								label={null}
								revealOnHover={false}
								class="rounded-md border bg-background px-2 py-1 text-xs"
								errorMessage={t('projects.domain_dns.copy_failed')} />
						</div>
					</div>

					<div class="overflow-hidden rounded-lg border">
						<Table.Root>
							<Table.Header>
								<Table.Row>
									<Table.Head class="w-16">{t('projects.projects.name')}</Table.Head>
									<Table.Head class="w-16">{t('projects.application_volumes.type')}</Table.Head>
									<Table.Head>{t('projects.domain_dns.value')}</Table.Head>
								</Table.Row>
							</Table.Header>
							<Table.Body>
								{#each ['@', '*'] as name (name)}
									<Table.Row>
										<Table.Cell class="font-mono">{name}</Table.Cell>
										<Table.Cell class="font-mono">{targetRecordType}</Table.Cell>
										<Table.Cell class="break-all font-mono">{target}</Table.Cell>
									</Table.Row>
								{/each}
							</Table.Body>
						</Table.Root>
					</div>
					<p class="text-xs text-muted-foreground">
						{t('projects.domain_dns.hint_1')} <code>@</code>
						{t('projects.domain_dns.hint_2')} <code>*</code>
						{t('projects.domain_dns.hint_3')}
					</p>
				{:else}
					<div class="rounded-lg border border-dashed p-3 text-sm text-muted-foreground">
						{t('projects.domain_dns.empty_1')} <code>@</code>
						{t('projects.domain_dns.empty_2')} <code>*</code>
						{t('projects.domain_dns.empty_3')}
					</div>
				{/if}

				<p class="text-xs text-muted-foreground">
					{t('projects.domain_dns.wildcard_hint')}
				</p>
			</div>
		{/if}
	</Dialog.Content>
</Dialog.Root>
