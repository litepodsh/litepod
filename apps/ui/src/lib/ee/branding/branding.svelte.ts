import { getBranding, type Branding } from "./branding-client";

const DEFAULT_NAME = "litepod";

export const branding = $state<Branding>({
  name: null,
  logo_url: null,
  favicon_url: null,
});

export const brandName = () => branding.name ?? DEFAULT_NAME;

/** "Home · Acme", or the bare name when the page has no title of its own. */
export function brandedTitle(page?: string, separator = "·"): string {
  return page ? `${page} ${separator} ${brandName()}` : brandName();
}

export function applyBranding(next: Branding) {
  Object.assign(branding, next);
}

/** Cloud answers with the defaults; on a failure the defaults stay too. */
export async function loadBranding() {
  try {
    applyBranding(await getBranding());
  } catch {
    // keep the Litepod defaults
  }
}
