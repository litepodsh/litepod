import { describe, expect, it } from 'vitest';
import { hasActiveCloudSubscription, shouldLoadCloudServerBilling } from './cloud-billing';

describe('shouldLoadCloudServerBilling', () => {
	it('does not load Billing for installation SysAdmin', () => {
		expect(shouldLoadCloudServerBilling(true, true)).toBe(false);
	});

	it('loads Billing for regular cloud users', () => {
		expect(shouldLoadCloudServerBilling(true, false)).toBe(true);
	});

	it('does not load Billing outside cloud mode', () => {
		expect(shouldLoadCloudServerBilling(false, false)).toBe(false);
	});
});

describe('hasActiveCloudSubscription', () => {
	it('accepts active subscriptions', () => {
		expect(hasActiveCloudSubscription('active')).toBe(true);
	});

	it('accepts trial subscriptions', () => {
		expect(hasActiveCloudSubscription('trialing')).toBe(true);
	});

	it.each(['inactive', 'past_due', 'paused', 'canceled', 'cancelled'])(
		'rejects %s subscriptions',
		(status) => {
			expect(hasActiveCloudSubscription(status)).toBe(false);
		},
	);
});
