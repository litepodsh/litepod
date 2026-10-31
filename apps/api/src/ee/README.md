# Enterprise features (API)

Everything under `ee/` is covered by the [Litepod Enterprise License](../../../../LICENSE_EE.md), not Apache 2.0. Production use needs a subscription. There is no license gate in code yet.

These routes are registered only when `!state.is_cloud` (`sso_routes_for_edition` in `router/router.rs`). On Litepod Cloud they return 404.

The matching panels live in [`apps/ui/src/lib/ee`](../../../ui/src/lib/ee/README.md).

| Feature | Handlers | Logic | Migrations |
| --- | --- | --- | --- |
| Single sign-on: OIDC, SAML, group mappings, verified domains, SCIM | `handler/sso.rs` | `sso/` | `sso_*` tables |
| White-label: instance name, logo and favicon | `handler/branding.rs` | none | `0072_instance_branding.sql` |

## Adding a feature here

1. Put the handlers in `handler/` and any logic in a folder next to it.
2. Register the routes in `router/router.rs` next to the other `ee` routes, so cloud never serves them.
3. Add the feature to the table above and to the list in the root `README.md`.
