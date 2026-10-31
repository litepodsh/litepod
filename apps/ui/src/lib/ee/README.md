# Enterprise features (UI)

Everything under `ee/` is covered by the [Litepod Enterprise License](../../../../../LICENSE_EE.md), not Apache 2.0. Production use needs a subscription.

These panels are for self-hosted instances. On Litepod Cloud the API has no endpoints for them, so they stay hidden or fall back to the defaults.

The matching API code lives in [`apps/api/src/ee`](../../../../api/src/ee/README.md).

| Feature | Where | Shown in |
| --- | --- | --- |
| Single sign-on and SCIM | `sso/` | Organization page, login page |
| White-label: name, logo, favicon | `branding/` | System → Branding; applied in the root layout, header and login page |

## Adding a feature here

1. Put the client, state and panel in their own folder.
2. Hide the entry point on cloud (`getRegistrationStatus().is_cloud`).
3. Add the feature to the table above and to the list in the root `README.md`.
