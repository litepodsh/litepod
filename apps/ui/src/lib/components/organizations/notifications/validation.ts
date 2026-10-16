import type {
	CatalogProvider,
	ProviderConfigInput,
	ProviderKind,
} from '#lib/clients/notification-client.js';

/**
 * Client-side mirror of `api/src/notifications/url_guard.rs`. The server is
 * still the authority — this exists so a mismatched webhook is caught while
 * the user is looking at the field, not after a round trip.
 */
const HOST_RULES: Record<ProviderKind, { exact: string[]; suffix: string[] }> = {
	slack: { exact: ['hooks.slack.com'], suffix: [] },
	discord: {
		exact: ['discord.com', 'discordapp.com'],
		suffix: ['.discord.com', '.discordapp.com'],
	},
	msteams: { exact: [], suffix: ['.webhook.office.com', '.logic.azure.com', '.logic.azure.us'] },
	telegram: { exact: ['api.telegram.org'], suffix: [] },
};

const PLACEHOLDERS: Record<ProviderKind, string> = {
	slack: 'https://hooks.slack.com/services/T0000/B0000/XXXXXXXX',
	discord: 'https://discord.com/api/webhooks/123456789/ABCDEFGHIJKLMNOPQRSTUVWXYZ',
	msteams: 'https://contoso.webhook.office.com/webhookb2/...',
	telegram: 'https://api.telegram.org',
};

export function hostHint(kind: ProviderKind): string {
	return PLACEHOLDERS[kind];
}

/** Which provider a URL actually belongs to, for a useful mismatch message. */
function detectKind(host: string): ProviderKind | null {
	for (const [kind, rule] of Object.entries(HOST_RULES) as [
		ProviderKind,
		(typeof HOST_RULES)[ProviderKind],
	][]) {
		if (rule.exact.includes(host) || rule.suffix.some((suffix) => host.endsWith(suffix))) {
			return kind;
		}
	}
	return null;
}

function validateWebhookUrl(
	kind: ProviderKind,
	raw: string,
	providers: CatalogProvider[],
): string | null {
	const value = raw.trim();
	if (!value) return 'Webhook URL is required';

	let url: URL;
	try {
		url = new URL(value);
	} catch {
		return 'That is not a valid URL';
	}

	if (url.protocol !== 'https:') return 'Webhook URL must use https';
	if (url.username || url.password) return 'Webhook URL must not contain credentials';
	if (url.port) return 'Webhook URL must not specify a port';

	const host = url.hostname.toLowerCase();
	const rule = HOST_RULES[kind];
	const allowed = rule.exact.includes(host) || rule.suffix.some((suffix) => host.endsWith(suffix));
	if (allowed) return null;

	const actual = detectKind(host);
	const label = providers.find((entry) => entry.kind === kind)?.label ?? kind;
	if (actual && actual !== kind) {
		const actualLabel = providers.find((entry) => entry.kind === actual)?.label ?? actual;
		return `That looks like a ${actualLabel} webhook, but ${label} is selected`;
	}
	const allowedHosts =
		providers.find((entry) => entry.kind === kind)?.allowed_hosts ??
		[...rule.exact, ...rule.suffix.map((suffix) => `*${suffix}`)].join(', ');
	return `${label} webhooks must point at ${allowedHosts} — got ${host}`;
}

function validateBotToken(raw: string): string | null {
	const value = raw.trim();
	if (!value) return 'Bot token is required';
	const [id, secret, ...rest] = value.split(':');
	const wellFormed =
		rest.length === 0 &&
		/^\d+$/.test(id ?? '') &&
		(secret?.length ?? 0) >= 20 &&
		/^[A-Za-z0-9_-]+$/.test(secret ?? '');
	return wellFormed
		? null
		: 'Bot token looks wrong — expected the form 123456789:AA... from @BotFather';
}

function validateChatId(raw: string): string | null {
	const value = raw.trim();
	if (!value) return 'Chat ID is required';
	const valid = value.startsWith('@') ? /^@[A-Za-z0-9_]{3,}$/.test(value) : /^-?\d+$/.test(value);
	return valid ? null : 'Chat ID must be a number (e.g. -1001234567890) or a @channelname';
}

/** First error found, or `null` when the config would be accepted. */
export function validateConfig(
	kind: ProviderKind,
	config: ProviderConfigInput,
	providers: CatalogProvider[],
): string | null {
	if (kind === 'telegram') {
		return validateBotToken(config.bot_token ?? '') ?? validateChatId(config.chat_id ?? '');
	}
	return validateWebhookUrl(kind, config.webhook_url ?? '', providers);
}
