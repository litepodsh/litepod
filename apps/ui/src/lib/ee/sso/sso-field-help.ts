/**
 * What each SSO field is called, and where it lives, in the common identity
 * providers. Names are the IdPs' own English UI labels, so they stay
 * untranslated; the explanation of each field lives in the i18n catalog under
 * `organizations.sso.help.<field>`.
 */
export type SsoHelpField =
	| 'name'
	| 'protocol'
	| 'issuer'
	| 'client_id'
	| 'client_secret'
	| 'groups_claim'
	| 'default_role'
	| 'jit'
	| 'enabled'
	| 'idp_metadata'
	| 'redirect_uri'
	| 'acs_url'
	| 'sp_entity_id'
	| 'session_ttl'
	| 'post_logout_redirect_uri'
	| 'backchannel_logout_url'
	| 'external_group'
	| 'tenant_url'
	| 'secret_token';

export interface IdpEquivalent {
	idp: string;
	/** The field's name in that IdP, or the value to use. */
	name: string;
	/** Where to find it, as a menu path. */
	where?: string;
}

const ENTRA = 'Microsoft Entra ID';
const OKTA = 'Okta';
const GOOGLE = 'Google Workspace';
const KEYCLOAK = 'Keycloak';

export const SSO_FIELD_EQUIVALENTS: Partial<Record<SsoHelpField, IdpEquivalent[]>> = {
	issuer: [
		{
			idp: ENTRA,
			name: 'https://login.microsoftonline.com/<Directory (tenant) ID>/v2.0',
			where: 'App registrations → app → Overview → Directory (tenant) ID',
		},
		{
			idp: OKTA,
			name: 'https://<org>.okta.com',
			where: 'Security → API → Authorization Servers → Issuer',
		},
		{ idp: GOOGLE, name: 'https://accounts.google.com' },
		{ idp: KEYCLOAK, name: 'https://<host>/realms/<realm>' },
	],
	client_id: [
		{
			idp: ENTRA,
			name: 'Application (client) ID',
			where: 'App registrations → app → Overview',
		},
		{ idp: OKTA, name: 'Client ID', where: 'Applications → app → General → Client Credentials' },
		{
			idp: GOOGLE,
			name: 'Client ID',
			where: 'Google Cloud console → APIs & Services → Credentials',
		},
		{ idp: KEYCLOAK, name: 'Client ID', where: 'Clients → client → Settings' },
	],
	client_secret: [
		{
			idp: ENTRA,
			name: 'Client secret → Value',
			where: 'App registrations → app → Certificates & secrets → Client secrets',
		},
		{
			idp: OKTA,
			name: 'Client secret',
			where: 'Applications → app → General → Client Credentials',
		},
		{
			idp: GOOGLE,
			name: 'Client secret',
			where: 'Google Cloud console → APIs & Services → Credentials',
		},
		{ idp: KEYCLOAK, name: 'Client secret', where: 'Clients → client → Credentials' },
	],
	groups_claim: [
		{
			idp: `${ENTRA} (OIDC)`,
			name: 'groups',
			where:
				'App registrations → app → Token configuration → Add groups claim → Security groups, Group ID',
		},
		{
			idp: `${ENTRA} (SAML)`,
			name: 'http://schemas.microsoft.com/ws/2008/06/identity/claims/groups',
			where:
				'Enterprise applications → app → Single sign-on → Attributes & Claims → Add a group claim',
		},
		{
			idp: OKTA,
			name: 'groups',
			where: 'Security → API → Authorization Servers → Claims → Add Claim',
		},
		{
			idp: KEYCLOAK,
			name: 'groups',
			where: 'Client scopes → <client>-dedicated → Add mapper → Group Membership',
		},
	],
	idp_metadata: [
		{
			idp: ENTRA,
			name: 'Federation Metadata XML',
			where: 'Enterprise applications → app → Single sign-on → SAML Certificates → Download',
		},
		{
			idp: OKTA,
			name: 'Identity Provider metadata',
			where: 'Applications → app → Sign On → SAML 2.0',
		},
		{
			idp: GOOGLE,
			name: 'IdP metadata',
			where: 'Admin console → Apps → Web and mobile apps → Add custom SAML app → Download metadata',
		},
		{ idp: KEYCLOAK, name: 'https://<host>/realms/<realm>/protocol/saml/descriptor' },
	],
	redirect_uri: [
		{ idp: ENTRA, name: 'Redirect URI (Web)', where: 'App registrations → app → Authentication' },
		{ idp: OKTA, name: 'Sign-in redirect URIs', where: 'Applications → app → General' },
		{
			idp: GOOGLE,
			name: 'Authorized redirect URIs',
			where: 'Google Cloud console → Credentials → OAuth client',
		},
		{ idp: KEYCLOAK, name: 'Valid redirect URIs', where: 'Clients → client → Settings' },
	],
	acs_url: [
		{
			idp: ENTRA,
			name: 'Reply URL (Assertion Consumer Service URL)',
			where: 'Enterprise applications → app → Single sign-on → Basic SAML Configuration',
		},
		{
			idp: OKTA,
			name: 'Single sign-on URL',
			where: 'Applications → app → General → SAML Settings',
		},
		{ idp: GOOGLE, name: 'ACS URL', where: 'Custom SAML app → Service provider details' },
		{
			idp: KEYCLOAK,
			name: 'Assertion Consumer Service POST Binding URL',
			where: 'Clients → client → Advanced',
		},
	],
	post_logout_redirect_uri: [
		{
			idp: ENTRA,
			name: 'Front-channel logout URL',
			where: 'App registrations → app → Authentication',
		},
		{ idp: OKTA, name: 'Sign-out redirect URIs', where: 'Applications → app → General' },
		{
			idp: KEYCLOAK,
			name: 'Valid post logout redirect URIs',
			where: 'Clients → client → Settings',
		},
	],
	backchannel_logout_url: [
		{
			idp: KEYCLOAK,
			name: 'Backchannel logout URL',
			where: 'Clients → client → Settings → Logout settings',
		},
	],
	sp_entity_id: [
		{
			idp: ENTRA,
			name: 'Identifier (Entity ID)',
			where: 'Enterprise applications → app → Single sign-on → Basic SAML Configuration',
		},
		{
			idp: OKTA,
			name: 'Audience URI (SP Entity ID)',
			where: 'Applications → app → General → SAML Settings',
		},
		{ idp: GOOGLE, name: 'Entity ID', where: 'Custom SAML app → Service provider details' },
		{ idp: KEYCLOAK, name: 'Client ID', where: 'Clients → SAML client → Settings' },
	],
	external_group: [
		{ idp: ENTRA, name: 'Object ID', where: 'Groups → group → Overview' },
		{ idp: OKTA, name: 'Group name', where: 'Directory → Groups' },
		{ idp: KEYCLOAK, name: 'Group name', where: 'Groups' },
	],
	tenant_url: [
		{
			idp: ENTRA,
			name: 'Tenant URL',
			where: 'Enterprise applications → app → Provisioning → Admin Credentials',
		},
		{
			idp: OKTA,
			name: 'SCIM connector base URL',
			where: 'Applications → app → Provisioning → Integration',
		},
	],
	secret_token: [
		{
			idp: ENTRA,
			name: 'Secret Token',
			where: 'Enterprise applications → app → Provisioning → Admin Credentials',
		},
		{
			idp: OKTA,
			name: 'Authentication Mode: HTTP Header → Bearer',
			where: 'Applications → app → Provisioning → Integration',
		},
	],
};
