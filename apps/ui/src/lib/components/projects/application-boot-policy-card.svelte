<script lang="ts">
	import * as Card from '#lib/components/ui/card/index.js';
	import * as Select from '#lib/components/ui/select/index.js';
	import PowerIcon from '@lucide/svelte/icons/power';
	import { getBootPolicy, updateBootPolicy, type BootPolicy } from '#lib/clients/project-client.js';
	import { toast, toastError } from '#lib/utils/toast.js';
	import { t } from '#lib/i18n/index.svelte.js';

	let {
		organizationId,
		projectId,
		environmentId,
		applicationId,
	}: {
		organizationId: string;
		projectId: string;
		environmentId: string;
		applicationId: string;
	} = $props();

	const policies: BootPolicy[] = ['restore', 'always', 'notify'];
	const labels = $derived<Record<BootPolicy, string>>({
		restore: t('projects.boot_policy.restore'),
		always: t('projects.boot_policy.always'),
		notify: t('projects.boot_policy.notify'),
	});
	const hints = $derived<Record<BootPolicy, string>>({
		restore: t('projects.boot_policy.restore_hint'),
		always: t('projects.boot_policy.always_hint'),
		notify: t('projects.boot_policy.notify_hint'),
	});

	let policy = $state<BootPolicy | null>(null);
	let saving = $state(false);

	$effect(() => {
		getBootPolicy(organizationId, projectId, environmentId, applicationId)
			.then((next) => (policy = next))
			.catch((err) => toastError(err, t('projects.boot_policy.save_failed')));
	});

	async function save(next: BootPolicy) {
		if (next === policy) return;
		const previous = policy;
		policy = next;
		saving = true;
		try {
			policy = await updateBootPolicy(
				organizationId,
				projectId,
				environmentId,
				applicationId,
				next,
			);
			toast.success(t('projects.boot_policy.saved'));
		} catch (err) {
			policy = previous;
			toastError(err, t('projects.boot_policy.save_failed'));
		} finally {
			saving = false;
		}
	}
</script>

<Card.Root class="max-w-3xl overflow-hidden">
	<Card.Header>
		<div class="flex items-center gap-2">
			<PowerIcon class="size-4 text-muted-foreground" />
			<Card.Title>{t('projects.boot_policy.title')}</Card.Title>
		</div>
		<Card.Description>{t('projects.boot_policy.description')}</Card.Description>
	</Card.Header>
	<Card.Content class="grid gap-2 pt-2">
		{#if policy}
			<Select.Root
				type="single"
				value={policy}
				disabled={saving}
				onValueChange={(value) => void save(value as BootPolicy)}>
				<Select.Trigger class="w-full">{labels[policy]}</Select.Trigger>
				<Select.Content>
					{#each policies as option (option)}
						<Select.Item value={option}>{labels[option]}</Select.Item>
					{/each}
				</Select.Content>
			</Select.Root>
			<p class="text-sm text-muted-foreground">{hints[policy]}</p>
		{/if}
	</Card.Content>
</Card.Root>
