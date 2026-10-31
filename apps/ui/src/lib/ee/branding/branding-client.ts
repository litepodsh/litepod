import { api } from "#lib/clients/base-client.js";

/** Enterprise white-label, self-hosted only. `null` means the Litepod default. */

export type Branding = {
  name: string | null;
  logo_url: string | null;
  favicon_url: string | null;
};

export async function getBranding(): Promise<Branding> {
  const response = await api<Branding>("/api/v1/branding");
  return response.data!;
}

export async function updateBranding(branding: Branding): Promise<Branding> {
  const response = await api<Branding>("/api/v1/settings/branding", {
    method: "PUT",
    body: JSON.stringify(branding),
  });
  return response.data!;
}
