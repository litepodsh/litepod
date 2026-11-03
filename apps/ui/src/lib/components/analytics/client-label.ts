import { t } from '#lib/i18n/index.svelte.js';

const BROWSERS: Record<string, string> = {
	chrome: 'Chrome',
	safari: 'Safari',
	firefox: 'Firefox',
	edge: 'Edge',
	opera: 'Opera',
};

/** Display name of a `client` value: a browser, `bot`, `tool`, `unknown` or anything else. */
export function clientLabel(key: string): string {
	if (BROWSERS[key]) return BROWSERS[key];
	if (key === 'bot' || key === 'tool' || key === 'unknown') return t(`analytics.client.${key}`);
	return t('analytics.client.other');
}
