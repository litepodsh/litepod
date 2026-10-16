/** Reject icon URLs the browser can't safely load in an app context
 *  (e.g. `file:///…` pasted from a local machine → Firefox "Security Error"). */
export function isDisplayableImageUrl(url: string | null | undefined): boolean {
	return !!url && /^(https?:\/\/|\/|data:image\/)/i.test(url.trim());
}

export function displayApplicationIconUrl(
	applicationId: string,
	iconUrl: string | null,
	catalogIconUrl: string | null,
	failedApplicationIds: ReadonlySet<string>,
): string | null {
	if (failedApplicationIds.has(applicationId)) return null;
	if (isDisplayableImageUrl(iconUrl)) return iconUrl;
	return isDisplayableImageUrl(catalogIconUrl) ? catalogIconUrl : null;
}
