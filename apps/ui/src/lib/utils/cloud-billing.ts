export function shouldLoadCloudServerBilling(cloudMode: boolean, isSysadmin: boolean): boolean {
	return cloudMode && !isSysadmin;
}

export function hasActiveCloudSubscription(status: string): boolean {
	return status === 'active' || status === 'trialing';
}
